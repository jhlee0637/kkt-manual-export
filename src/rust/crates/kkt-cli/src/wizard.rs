//! 인자 없이 실행(더블클릭)하면 뜨는 안내 마당. 열려 있는 카카오톡 방을 고르면 수집부터 정리까지 한 번에 한다.
//!
//! 설정(저장 폴더, 카카오톡 사진 저장 폴더, 동영상 보관)은 설정 파일에 기억하며, 파일 위치는 화면 맨 위에 보여 준다.
//! 저장 폴더 아래 `archive/` 에 정리 결과, `exports/` 에 내보내기 TXT 를 둔다.

use crate::settings::{self, LoadStatus, Settings};
use kkt_core::archive::Archive;
use kkt_core::attach::ingest_attachments;
use kkt_win::{collect, guard::Guard, photos, sys, window, WinError};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

/// 방 하나를 처리한 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// 사용자가 Ctrl+D 로 중단했다. 남은 방은 건너뛴다.
    Aborted,
    Failed,
}

fn archive_dir(s: &Settings) -> PathBuf {
    s.output_dir.join("archive")
}

fn exports_dir(s: &Settings) -> PathBuf {
    s.output_dir.join("exports")
}

/// 입력한 번호들을 목록 위치(0부터)로 바꾼다. `1,3` `1-3` `1 3` `a`(전체) 를 받는다. 중복은 처음 것만 남긴다.
pub fn parse_selection(line: &str, n: usize) -> Result<Vec<usize>, String> {
    let mut out: Vec<usize> = Vec::new();
    let mut push = |i: usize| {
        if !out.contains(&i) {
            out.push(i);
        }
    };
    for tok in line.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()) {
        let low = tok.to_lowercase();
        if low == "a" || low == "all" || tok == "전체" {
            (0..n).for_each(&mut push);
            continue;
        }
        let range = |a: &str, b: &str| -> Option<(usize, usize)> { Some((a.parse().ok()?, b.parse().ok()?)) };
        let (lo, hi) = match tok.split_once('-') {
            Some((a, b)) => range(a, b).ok_or_else(|| format!("잘못된 입력입니다: {tok:?}"))?,
            None => {
                let k: usize = tok.parse().map_err(|_| format!("잘못된 입력입니다: {tok:?}"))?;
                (k, k)
            }
        };
        if lo == 0 || hi < lo || hi > n {
            return Err(format!("번호는 1 ~ {n} 사이여야 합니다: {tok:?}"));
        }
        (lo..=hi).for_each(|k| push(k - 1));
    }
    if out.is_empty() {
        return Err("번호를 입력하세요".to_string());
    }
    Ok(out)
}

/// 입력한 줄을 목록 번호(0부터)로 바꾼다. 빈 입력과 잘못된 입력은 `None`.
pub fn parse_choice(line: &str, n: usize) -> Option<usize> {
    let k: usize = line.trim().parse().ok()?;
    (1..=n).contains(&k).then(|| k - 1)
}

fn count(events: &Value, key: &str) -> u64 {
    events.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// 반영 결과(`ingest` 가 낸 JSON)를 사람이 읽을 문장으로 바꾼다.
pub fn summarize(result: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if result.get("skipped").is_some() {
        out.push("이미 반영된 내보내기입니다. 달라진 것이 없습니다.".to_string());
        return out;
    }
    out.push(
        match result.get("link").and_then(Value::as_str) {
            Some("new") => "처음 보는 방이라 새로 기록했습니다.",
            Some("renamed_room") => "방 이름이 바뀐 것으로 판단해 기존 기록에 이어 붙였습니다.",
            _ => "기존 기록에 이어 붙였습니다.",
        }
        .to_string(),
    );
    let ev = result.get("events").cloned().unwrap_or(Value::Null);
    let lines = [
        ("message.observed", "새 메시지"),
        ("message.deleted_for_everyone", "모두에게 삭제된 메시지"),
        ("deleted_marker.observed", "삭제 표식만 보이는 메시지"),
        ("message.missing", "사라진 메시지 (나에게서만 삭제했을 수 있음)"),
        ("message.edit_candidate", "수정된 것으로 보이는 메시지"),
        ("participant.renamed", "이름이 바뀐 참가자"),
    ];
    let mut any = false;
    for (key, label) in lines {
        let n = count(&ev, key);
        if n > 0 {
            out.push(format!("  {label}: {n}개"));
            any = true;
        }
    }
    if !any {
        out.push("  달라진 메시지가 없습니다.".to_string());
    }
    if let Some(ws) = result.get("warnings").and_then(Value::as_array) {
        for w in ws {
            out.push(format!("주의: {}", w.as_str().unwrap_or_default()));
        }
    }
    out
}

fn prompt(text: &str) -> Option<String> {
    print!("{text}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

/// 서랍에서 골라야 할 최신 타일 수와 그때 저장될 파일 수: `(타일, 파일)`.
///
/// 서랍은 사진과 동영상을 메시지 하나당 타일 하나로 최신순으로 보여 준다 (`사진 3장` 묶음도 타일 하나).
/// 아직 사진 파일과 연결되지 않은 가장 오래된 사진 메시지가 나올 때까지의 최신 사진·동영상 메시지가 대상이다.
/// 동영상도 타일을 차지하므로 함께 센다 (동영상 파일은 저장되지만 아카이브에 보관하지는 않는다).
/// 파일 수는 사진 메시지의 장수 합 + 동영상 수다.
fn media_to_fetch(archive: &Path, conv: &str) -> (usize, usize) {
    let Ok((st, _)) = Archive::new(archive, conv).load() else { return (0, 0) };
    let mut media: Vec<(&str, &str, usize, &kkt_core::state::Record)> = st
        .registry
        .values()
        .enumerate()
        .filter(|(_, r)| r.kind == "message" && r.status == "active" && (r.content_type == "image" || r.content_type == "video"))
        .map(|(i, r)| (r.date.as_str(), r.hhmm.as_deref().unwrap_or(""), i, r))
        .collect();
    media.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    let Some(oldest) = media.iter().position(|m| m.3.content_type == "image" && m.3.attachment_ids.is_empty()) else {
        return (0, 0);
    };
    let files = media[oldest..].iter().map(|m| if m.3.content_type == "image" { m.3.image_count as usize } else { 1 }).sum();
    (media.len() - oldest, files)
}

/// 서랍에서 최신 사진을 (연결 안 된 사진 메시지 수만큼) 저장하고 메시지와 연결한다.
fn save_photos(title: &str, settings: &Settings, conv: &str) {
    let archive = archive_dir(settings);
    let (msgs, n) = media_to_fetch(&archive, conv);
    if msgs == 0 {
        println!("저장할 새 사진이 없습니다.");
        return;
    }
    println!("\n사진·동영상 {n}개(서랍의 {msgs}칸)가 아직 저장되지 않았습니다. 서랍에서 저장합니다. 마우스와 키보드에서 손을 떼세요. (마우스를 움직이면 일시정지, 중단: Ctrl+D)");
    let dir = settings.kakao_photo_dir.clone();
    let opt = photos::Options { newest: Some(msgs), expect_files: Some(n), ..Default::default() };
    match photos::download_photos(title, &dir, &Guard::new(), &opt) {
        Ok(meta) => {
            let saved = meta["saved_files"].as_array().map_or(0, |a| a.len());
            println!("사진 {saved}장을 저장했습니다: {}", dir.display());
            for w in meta["warnings"].as_array().into_iter().flatten() {
                println!("주의: {}", w.as_str().unwrap_or_default());
            }
            match ingest_attachments(&Archive::new(&archive, conv), &dir) {
                Ok(r) => {
                    println!("사진을 보관하고 메시지와 연결했습니다: 새로 보관 {}장, 연결 {}장", r["saved"], r["linked"]);
                    for g in r["unmatched_groups"].as_array().into_iter().flatten() {
                        println!(
                            "주의: {} 에 사진 메시지 {}개, 파일 {}개라서 연결하지 않았습니다 (추측하지 않습니다)",
                            g["minute"].as_str().unwrap_or_default(),
                            g["image_messages"],
                            g["files"]
                        );
                    }
                }
                Err(e) => println!("사진을 연결하지 못했습니다 [{}]: {}", e.code, e.message),
            }
        }
        Err(WinError::Aborted) => println!("\nCtrl+D 로 중단했습니다. 사진은 반영하지 않았습니다."),
        Err(e) => println!("\n사진을 저장하지 못했습니다: {e}"),
    }
}

fn collect_and_ingest(title: &str, settings: &Settings) -> Outcome {
    let archive = archive_dir(settings);
    println!("\n시작합니다. 마우스와 키보드에서 손을 떼세요. (마우스를 움직이면 일시정지, 중단: Ctrl+D)");
    std::thread::sleep(std::time::Duration::from_secs(2));
    let meta = match collect::export_chat(title, &exports_dir(settings), &Guard::new(), &collect::Options::default()) {
        Ok(m) => m,
        Err(WinError::Aborted) => {
            println!("\nCtrl+D 로 중단했습니다. 아무것도 반영하지 않았습니다.");
            return Outcome::Aborted;
        }
        Err(e) => {
            println!("\n수집하지 못했습니다: {e}");
            return Outcome::Failed;
        }
    };
    let path = PathBuf::from(meta["path"].as_str().unwrap_or_default());
    println!("\n내보내기를 저장했습니다: {}", path.display());
    for w in meta["warnings"].as_array().into_iter().flatten() {
        println!("주의: {}", w.as_str().unwrap_or_default());
    }
    match crate::ingest_path(&archive, None, &path, false, &indexmap::IndexMap::new()) {
        Ok(r) => {
            let conv = r.get("conversation").and_then(Value::as_str).map(str::to_string);
            if let Some(c) = &conv {
                println!("정리 위치: {}", archive.join(c).display());
            }
            for l in summarize(&r) {
                println!("{l}");
            }
            if let Some(c) = conv {
                save_photos(title, settings, &c);
            }
            Outcome::Done
        }
        Err(e) => {
            println!("정리하지 못했습니다 [{}]: {}", e.code, e.message);
            println!("내보낸 파일은 그대로 남아 있습니다. 문제가 계속되면 위 메시지를 알려 주세요.");
            Outcome::Failed
        }
    }
}

/// 끌어다 놓거나 붙여 넣은 경로를 정리한다 (앞뒤 따옴표, 공백 앞의 역슬래시, 줄 끝 공백).
pub fn clean_path(line: &str) -> String {
    let t = line.trim().trim_matches(|c| c == '\'' || c == '"');
    t.replace("\\ ", " ")
}

/// 카카오톡 내보내기 TXT 로 보이는 파일인지 (이름이 `KakaoTalk` 로 시작하는 `.txt`).
pub fn looks_like_export(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("kakaotalk") && n.ends_with(".txt")
}

/// 폴더들에서 내보내기 TXT 를 찾는다. 최근에 바뀐 것부터, 최대 `limit` 개. 정리 위치(`base`) 안의 파일은 뺀다.
pub fn find_exports(dirs: &[PathBuf], base: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.starts_with(base) || !p.is_file() {
                continue;
            }
            if !looks_like_export(&e.file_name().to_string_lossy()) {
                continue;
            }
            let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            found.push((t, p));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().take(limit).map(|f| f.1).collect()
}

fn search_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) else {
        return Vec::new();
    };
    ["Downloads", "Documents", "Desktop"].iter().map(|d| home.join(d)).collect()
}

fn ingest_and_report(path: &Path, settings: &Settings) {
    let archive = archive_dir(settings);
    println!("\n반영합니다: {}", path.display());
    match crate::ingest_path(&archive, None, path, false, &indexmap::IndexMap::new()) {
        Ok(r) => {
            if let Some(c) = r.get("conversation").and_then(Value::as_str) {
                println!("정리 위치: {}", archive.join(c).display());
            }
            for l in summarize(&r) {
                println!("{l}");
            }
        }
        Err(e) => println!("정리하지 못했습니다 [{}]: {}", e.code, e.message),
    }
}

/// 화면 맨 위에 보여 줄 줄들. 설정 파일 위치를 첫머리에 둔다.
pub fn header_lines(cfg_path: &Path, s: &Settings, status: &LoadStatus) -> Vec<String> {
    let mut v = vec!["== 카카오톡 대화 수집 ==".to_string(), format!("설정 파일: {}", cfg_path.display())];
    match status {
        LoadStatus::Created => v.push("  (처음 실행이라 기본 설정으로 새로 만들었습니다)".to_string()),
        LoadStatus::Invalid(why) => {
            v.push(format!("  주의: 설정 파일을 읽지 못해 이번에는 기본값을 씁니다 ({why}). 파일은 덮어쓰지 않았습니다."));
        }
        LoadStatus::Loaded => {}
    }
    v.push(format!("저장 폴더: {}  (정리 결과 archive, 내보내기 exports)", s.output_dir.display()));
    v.push(format!("카카오톡 사진 저장 폴더: {}", s.kakao_photo_dir.display()));
    v.push(format!("동영상: {}", s.videos.label()));
    v
}

/// 폴더를 탐색기(Finder)로 연다. 없으면 만든다.
pub fn open_folder(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("폴더를 만들지 못했습니다: {e}"))?;
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(path).spawn().map(|_| ()).map_err(|e| format!("{program} 를 실행하지 못했습니다: {e}"))
}

/// 설정 화면에서 입력한 경로를 정리한다. 빈 입력은 `None`(취소), 상대 경로는 현재 폴더 기준으로 바꾼다.
pub fn normalize_input_path(line: &str, cwd: &Path) -> Option<PathBuf> {
    let t = clean_path(line);
    if t.trim().is_empty() {
        return None;
    }
    let p = PathBuf::from(t.trim());
    Some(if p.is_absolute() { p } else { cwd.join(p) })
}

fn save_settings(s: &Settings, cfg_path: &Path) {
    match s.save(cfg_path) {
        Ok(()) => println!("저장했습니다: {}", cfg_path.display()),
        Err(e) => println!("설정을 저장하지 못했습니다 ({}): {e}", cfg_path.display()),
    }
}

/// 설정 화면. 바꾸는 즉시 저장한다.
fn settings_menu(s: &mut Settings, cfg_path: &Path) {
    loop {
        println!("\n-- 설정 (바꾸면 바로 저장됩니다) --");
        println!("설정 파일: {}", cfg_path.display());
        println!("  1) 저장 폴더: {}", s.output_dir.display());
        println!("  2) 카카오톡 사진 저장 폴더: {}", s.kakao_photo_dir.display());
        println!("     (카카오톡 설정의 '사진 저장 위치'와 같아야 저장된 사진을 확인할 수 있습니다. 이 프로그램은 그 폴더를 바꾸지 않고 지켜보기만 합니다)");
        println!("  3) 동영상: {}  (누를 때마다 바뀝니다)", s.videos.label());
        println!("  r) 기본값으로 되돌리기");
        let Some(line) = prompt("번호를 입력하세요 (Enter = 돌아가기): ") else { return };
        match line.trim().to_lowercase().as_str() {
            "" => return,
            "1" | "2" => {
                let which = line.trim() == "1";
                let Some(l) = prompt("새 폴더 경로 (Enter = 취소, 폴더를 이 창에 끌어다 놓아도 됩니다): ") else { return };
                let cwd = std::env::current_dir().unwrap_or_default();
                let Some(p) = normalize_input_path(&l, &cwd) else { continue };
                if which {
                    if let Err(e) = std::fs::create_dir_all(&p) {
                        println!("그 폴더를 만들 수 없습니다: {e}");
                        continue;
                    }
                    s.output_dir = p;
                } else {
                    if !p.is_dir() {
                        println!("주의: 아직 없는 폴더입니다. 카카오톡이 사진을 이 폴더에 저장하도록 설정되어 있는지 확인하세요.");
                    }
                    s.kakao_photo_dir = p;
                }
                save_settings(s, cfg_path);
            }
            "3" => {
                s.videos = s.videos.next();
                save_settings(s, cfg_path);
            }
            "r" => {
                *s = Settings::default();
                save_settings(s, cfg_path);
            }
            _ => println!("1, 2, 3, r 중에서 고르세요."),
        }
    }
}

/// Windows 가 아닐 때: 카카오톡 조작(수집)은 못 하므로 직접 내보낸 TXT 를 골라 정리만 한다.
fn files_menu(s: &mut Settings, cfg_path: &Path) -> bool {
    println!("\n이 컴퓨터에서는 카카오톡을 자동으로 조작할 수 없습니다 (수집은 Windows 전용).");
    println!("카카오톡에서 직접 대화를 내보낸 TXT 파일을 고르면 정리해 드립니다.\n");
    let files = find_exports(&search_dirs(), &s.output_dir, 20);
    if files.is_empty() {
        println!("다운로드·문서·바탕화면에서 내보내기 파일(KakaoTalk…txt)을 찾지 못했습니다.");
    } else {
        println!("찾은 내보내기 파일 (최근 순):");
        for (i, f) in files.iter().enumerate() {
            println!("  {}) {}", i + 1, f.display());
        }
    }
    let Some(line) = prompt("\n번호를 입력하거나 파일을 이 창에 끌어다 놓으세요 (o = 결과 폴더 열기, s = 설정, Enter = 종료): ") else { return false };
    match line.trim().to_lowercase().as_str() {
        "" => return false,
        "o" => {
            if let Err(e) = open_folder(&s.output_dir) {
                println!("{e}");
            }
        }
        "s" => settings_menu(s, cfg_path),
        _ => {
            if let Some(i) = parse_choice(&line, files.len()) {
                ingest_and_report(&files[i], s);
            } else {
                let p = PathBuf::from(clean_path(&line));
                if p.is_file() {
                    ingest_and_report(&p, s);
                } else {
                    println!("\n번호나 올바른 파일 경로가 아닙니다.");
                }
            }
        }
    }
    true
}

/// Windows: 열려 있는 방 목록에서 골라 수집한다. 계속하려면 true.
fn rooms_menu(s: &mut Settings, cfg_path: &Path) -> bool {
    let titles = window::list_chat_titles();
    if titles.is_empty() {
        println!("\n열려 있는 채팅방이 없습니다. 수집할 방을 카카오톡에서 창으로 열어 둔 뒤 Enter 를 누르세요. (o = 결과 폴더 열기, s = 설정, q = 종료)");
        let Some(line) = prompt("") else { return false };
        match line.trim().to_lowercase().as_str() {
            "q" => return false,
            "o" => {
                let _ = open_folder(&s.output_dir).map_err(|e| println!("{e}"));
            }
            "s" => settings_menu(s, cfg_path),
            _ => {}
        }
        return true;
    }
    println!("\n열려 있는 채팅방:");
    for (i, t) in titles.iter().enumerate() {
        println!("  {}) {t}", i + 1);
    }
    let Some(line) = prompt("\n수집할 방 번호 (여러 개는 1,3 또는 1-3, a = 모두) / o = 결과 폴더 열기 / s = 설정 / Enter = 종료: ") else { return false };
    match line.trim().to_lowercase().as_str() {
        "" => return false,
        "o" => {
            if let Err(e) = open_folder(&s.output_dir) {
                println!("{e}");
            }
        }
        "s" => settings_menu(s, cfg_path),
        _ => match parse_selection(&line, titles.len()) {
            Ok(picked) => collect_rooms(&titles, &picked, s),
            Err(msg) => println!("\n{msg}"),
        },
    }
    true
}

/// 고른 방들을 차례로 처리한다. Ctrl+D 로 중단하면 남은 방은 건너뛴다.
fn collect_rooms(titles: &[String], picked: &[usize], s: &Settings) {
    let (mut done, mut failed, mut skipped) = (0, 0, 0);
    for (k, &i) in picked.iter().enumerate() {
        if picked.len() > 1 {
            println!("\n===== [{}/{}] {} =====", k + 1, picked.len(), titles[i]);
        }
        match collect_and_ingest(&titles[i], s) {
            Outcome::Done => done += 1,
            Outcome::Failed => failed += 1,
            Outcome::Aborted => {
                skipped = picked.len() - k - 1;
                failed += 1;
                break;
            }
        }
    }
    if picked.len() > 1 {
        println!("\n전체 결과: 완료 {done}개, 실패·중단 {failed}개{}", if skipped > 0 { format!(", 건너뜀 {skipped}개") } else { String::new() });
    }
    println!();
}

pub fn run() -> i32 {
    sys::set_console_utf8();
    let cfg_path = settings::config_path();
    let (mut s, status) = Settings::load(&cfg_path);
    loop {
        for l in header_lines(&cfg_path, &s, &status) {
            println!("{l}");
        }
        let go_on = if sys::SUPPORTED { rooms_menu(&mut s, &cfg_path) } else { files_menu(&mut s, &cfg_path) };
        if !go_on {
            return 0;
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn choice_parsing() {
        assert_eq!(parse_choice("1\n", 3), Some(0));
        assert_eq!(parse_choice(" 3 ", 3), Some(2));
        assert_eq!(parse_choice("0", 3), None);
        assert_eq!(parse_choice("4", 3), None);
        assert_eq!(parse_choice("", 3), None);
        assert_eq!(parse_choice("가", 3), None);
    }

    #[test]
    fn selection_parsing() {
        assert_eq!(parse_selection("1", 3).unwrap(), [0]);
        assert_eq!(parse_selection("1,3", 3).unwrap(), [0, 2]);
        assert_eq!(parse_selection(" 3 1 ", 3).unwrap(), [2, 0], "입력한 순서를 지킨다");
        assert_eq!(parse_selection("1-3", 3).unwrap(), [0, 1, 2]);
        assert_eq!(parse_selection("a", 3).unwrap(), [0, 1, 2]);
        assert_eq!(parse_selection("ALL", 2).unwrap(), [0, 1]);
        assert_eq!(parse_selection("전체", 2).unwrap(), [0, 1]);
        assert_eq!(parse_selection("2,2,1-2", 3).unwrap(), [1, 0], "중복은 처음 것만");
        for bad in ["", "  ", "0", "4", "1-4", "3-1", "x", "1,x", "-1", "1-", "a1"] {
            assert!(parse_selection(bad, 3).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn header_puts_config_path_first() {
        let cfg = Path::new("/home/a/.config/kkt-manual-export/config.json");
        let s = Settings::default();
        let h = header_lines(cfg, &s, &LoadStatus::Loaded);
        assert_eq!(h[0], "== 카카오톡 대화 수집 ==");
        assert!(h[1].starts_with("설정 파일: ") && h[1].contains("config.json"), "{:?}", h[1]);
        assert!(h.iter().any(|l| l.contains("저장 폴더")) && h.iter().any(|l| l.contains("동영상")));
        let created = header_lines(cfg, &s, &LoadStatus::Created);
        assert!(created[2].contains("새로 만들었습니다"));
        let bad = header_lines(cfg, &s, &LoadStatus::Invalid("expected value".into()));
        assert!(bad[2].contains("덮어쓰지 않았습니다") && bad[2].contains("expected value"));
    }

    #[test]
    fn input_path_normalization() {
        let cwd = Path::new("/work");
        assert_eq!(normalize_input_path("", cwd), None);
        assert_eq!(normalize_input_path("   \n", cwd), None);
        assert_eq!(normalize_input_path("'/data/My\\ Files/'", cwd), Some(PathBuf::from("/data/My Files/")));
        assert_eq!(normalize_input_path("rel/dir", cwd), Some(PathBuf::from("/work/rel/dir")));
    }

    #[test]
    fn path_cleanup() {
        assert_eq!(clean_path("  '/Users/a/My\\ Files/KakaoTalk_1.txt' \n"), "/Users/a/My Files/KakaoTalk_1.txt");
        assert_eq!(clean_path("\"C:\\x\\y.txt\""), "C:\\x\\y.txt");
    }

    #[test]
    fn export_name_detection() {
        assert!(looks_like_export("KakaoTalk_20261002_1641_group.txt"));
        assert!(looks_like_export("kakaotalk chat.TXT"));
        assert!(!looks_like_export("notes.txt"));
        assert!(!looks_like_export("KakaoTalk_1.png"));
    }

    #[test]
    fn find_exports_orders_and_excludes_base() {
        let root = std::env::temp_dir().join(format!("kkt-wiz-{}", std::process::id()));
        let base = root.join("dl").join(settings::FOLDER_NAME);
        std::fs::create_dir_all(&base).unwrap();
        let dl = root.join("dl");
        std::fs::write(dl.join("KakaoTalk_a.txt"), "a").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(dl.join("KakaoTalk_b.txt"), "b").unwrap();
        std::fs::write(dl.join("other.txt"), "x").unwrap();
        std::fs::write(base.join("KakaoTalk_in_base.txt"), "z").unwrap();
        let v = find_exports(&[dl.clone(), root.join("missing")], &base, 10);
        let names: Vec<_> = v.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["KakaoTalk_b.txt", "KakaoTalk_a.txt"]);
        assert_eq!(find_exports(&[dl], &base, 1).len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn summary_skipped() {
        let s = summarize(&json!({"skipped": "이미 반영된 내보내기", "events": {}}));
        assert_eq!(s.len(), 1);
        assert!(s[0].contains("이미 반영"));
    }

    #[test]
    fn summary_counts_and_warnings() {
        let s = summarize(&json!({"link": "same",
            "events": {"message.observed": 3, "message.missing": 1}, "warnings": ["이름 확인 필요"]}));
        assert_eq!(s[0], "기존 기록에 이어 붙였습니다.");
        assert!(s.contains(&"  새 메시지: 3개".to_string()));
        assert!(s.iter().any(|l| l.contains("사라진 메시지") && l.contains("1개")));
        assert!(s.contains(&"주의: 이름 확인 필요".to_string()));
    }

    #[test]
    fn summary_no_changes_and_new_room() {
        let s = summarize(&json!({"link": "new", "events": {}, "warnings": []}));
        assert_eq!(s[0], "처음 보는 방이라 새로 기록했습니다.");
        assert!(s[1].contains("달라진 메시지가 없습니다"));
    }
}
