//! 인자 없이 실행(더블클릭)하면 뜨는 안내 마당. 열려 있는 카카오톡 방을 고르면 수집부터 정리까지 한 번에 한다.
//!
//! 저장 위치는 `다운로드\kkt-manual-export-archive` (`archive/` 에 정리 결과, `exports/` 에 내보내기 TXT).

use kkt_core::archive::Archive;
use kkt_core::attach::ingest_attachments;
use kkt_win::{collect, guard::Guard, photos, sys, window, WinError};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub const FOLDER_NAME: &str = "kkt-manual-export-archive";

/// `다운로드\kkt-manual-export-archive`. 홈 폴더를 알 수 없으면 현재 폴더 아래.
pub fn base_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    home.map(|h| h.join("Downloads")).unwrap_or_default().join(FOLDER_NAME)
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

/// 아직 사진 파일과 연결되지 않은, 지금 보이는 사진 수 ('사진 3장' 한 줄은 3장).
fn unlinked_images(archive: &Path, conv: &str) -> usize {
    match Archive::new(archive, conv).load() {
        Ok((st, _)) => st
            .registry
            .values()
            .filter(|r| r.kind == "message" && r.content_type == "image" && r.status == "active" && r.attachment_ids.is_empty())
            .map(|r| r.image_count as usize)
            .sum(),
        Err(_) => 0,
    }
}

/// 서랍에서 최신 사진을 (연결 안 된 사진 메시지 수만큼) 저장하고 메시지와 연결한다.
fn save_photos(title: &str, archive: &Path, conv: &str) {
    let n = unlinked_images(archive, conv);
    if n == 0 {
        println!("저장할 새 사진이 없습니다.");
        return;
    }
    println!("\n사진 {n}장이 아직 저장되지 않았습니다. 서랍에서 저장합니다. 마우스와 키보드에서 손을 떼세요. (중단: Ctrl+D)");
    let dir = photos::default_save_dir();
    let opt = photos::Options { newest: Some(n), ..Default::default() };
    match photos::download_photos(title, &dir, &Guard::new(), &opt) {
        Ok(meta) => {
            let saved = meta["saved_files"].as_array().map_or(0, |a| a.len());
            println!("사진 {saved}장을 저장했습니다: {}", dir.display());
            for w in meta["warnings"].as_array().into_iter().flatten() {
                println!("주의: {}", w.as_str().unwrap_or_default());
            }
            match ingest_attachments(&Archive::new(archive, conv), &dir) {
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

fn collect_and_ingest(title: &str, base: &Path) {
    let exports = base.join("exports");
    let archive = base.join("archive");
    println!("\n시작합니다. 몇 초 동안 마우스와 키보드에서 손을 떼세요. (중단: Ctrl+D)");
    std::thread::sleep(std::time::Duration::from_secs(2));
    let meta = match collect::export_chat(title, &exports, &Guard::new(), &collect::Options::default()) {
        Ok(m) => m,
        Err(WinError::Aborted) => {
            println!("\nCtrl+D 로 중단했습니다. 아무것도 반영하지 않았습니다.");
            return;
        }
        Err(e) => {
            println!("\n수집하지 못했습니다: {e}");
            return;
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
                save_photos(title, &archive, &c);
            }
        }
        Err(e) => {
            println!("정리하지 못했습니다 [{}]: {}", e.code, e.message);
            println!("내보낸 파일은 그대로 남아 있습니다. 문제가 계속되면 위 메시지를 알려 주세요.");
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

fn ingest_and_report(path: &Path, base: &Path) {
    let archive = base.join("archive");
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

/// Windows 가 아닐 때: 카카오톡 조작(수집)은 못 하므로 직접 내보낸 TXT 를 골라 정리만 한다.
fn run_files(base: &Path) -> i32 {
    println!("이 컴퓨터에서는 카카오톡을 자동으로 조작할 수 없습니다 (수집은 Windows 전용).");
    println!("카카오톡에서 직접 대화를 내보낸 TXT 파일을 고르면 정리해 드립니다.\n");
    loop {
        let files = find_exports(&search_dirs(), base, 20);
        if files.is_empty() {
            println!("다운로드·문서·바탕화면에서 내보내기 파일(KakaoTalk…txt)을 찾지 못했습니다.");
        } else {
            println!("찾은 내보내기 파일 (최근 순):");
            for (i, f) in files.iter().enumerate() {
                println!("  {}) {}", i + 1, f.display());
            }
        }
        let Some(line) = prompt("\n번호를 입력하거나 파일을 이 창에 끌어다 놓으세요 (Enter = 종료): ") else { return 0 };
        if line.trim().is_empty() {
            return 0;
        }
        if let Some(i) = parse_choice(&line, files.len()) {
            ingest_and_report(&files[i], base);
        } else {
            let p = PathBuf::from(clean_path(&line));
            if p.is_file() {
                ingest_and_report(&p, base);
            } else {
                println!("\n번호나 올바른 파일 경로가 아닙니다.");
            }
        }
        println!();
    }
}

pub fn run() -> i32 {
    sys::set_console_utf8();
    let base = base_dir();
    println!("== 카카오톡 대화 정리 ==");
    println!("저장 위치: {}\n", base.display());
    if !sys::SUPPORTED {
        return run_files(&base);
    }
    loop {
        let titles = window::list_chat_titles();
        if titles.is_empty() {
            println!("열려 있는 채팅방이 없습니다. 수집할 방을 카카오톡에서 창으로 열어 둔 뒤 Enter 를 누르세요. (그냥 닫으려면 창을 닫으세요)");
            if prompt("").is_none() {
                return 0;
            }
            continue;
        }
        println!("열려 있는 채팅방:");
        for (i, t) in titles.iter().enumerate() {
            println!("  {}) {t}", i + 1);
        }
        let Some(line) = prompt("\n수집할 방의 번호를 입력하세요 (Enter = 종료): ") else { return 0 };
        if line.trim().is_empty() {
            return 0;
        }
        match parse_choice(&line, titles.len()) {
            Some(i) => {
                collect_and_ingest(&titles[i], &base);
                println!();
            }
            None => println!("\n1 ~ {} 사이의 번호를 입력하세요.\n", titles.len()),
        }
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
    fn base_dir_ends_with_downloads_folder() {
        let p = base_dir();
        assert!(p.ends_with(Path::new("Downloads").join(FOLDER_NAME)));
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
        let base = root.join("dl").join(FOLDER_NAME);
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
