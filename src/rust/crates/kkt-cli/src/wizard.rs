//! 인자 없이 실행(더블클릭)하면 뜨는 안내 마당. 열려 있는 카카오톡 방을 고르면 수집부터 정리까지 한 번에 한다.
//!
//! 설정(저장 폴더, 카카오톡 사진 저장 폴더, 동영상 보관)은 설정 파일에 기억하며, 파일 위치는 화면 맨 위에 보여 준다.
//! 저장 폴더 아래 `archive/` 에 정리 결과, `exports/` 에 내보내기 TXT 를 둔다.

use crate::guide;
use crate::ingest_flow::{ingest_interactive, Asker, Flow};
use crate::media_plan::{self, MediaRec};
use crate::screen::{self, Row};
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
    /// 사용자가 반영하지 않기로 했다.
    Skipped,
    /// 사용자가 Ctrl+D 로 중단했다. 남은 방은 건너뛴다.
    Aborted,
    Failed,
}

/// 콘솔에서 번호로 묻는다. 입력이 없거나 Enter 면 `default`(항상 가장 안전한 선택).
struct ConsoleAsker;

impl Asker for ConsoleAsker {
    fn choose(&mut self, question: &str, options: &[String], default: usize) -> usize {
        println!("\n? {question}");
        for (i, o) in options.iter().enumerate() {
            println!("  {}) {o}", i + 1);
        }
        loop {
            let Some(line) = prompt(&format!("번호를 입력하세요 (Enter = {}번): ", default + 1)) else {
                println!();
                return default;
            };
            let t = line.trim();
            if t.is_empty() {
                return default;
            }
            match parse_choice(t, options.len()) {
                Some(i) => return i,
                None => println!("1 ~ {} 사이의 번호를 입력하세요.", options.len()),
            }
        }
    }
}

/// 내보내기 하나를 반영한다 (필요하면 묻는다). 반영했으면 방 ID 를 돌려준다.
fn ingest_and_summarize(path: &Path, settings: &Settings) -> (Outcome, Option<String>) {
    let archive = archive_dir(settings);
    match ingest_interactive(&archive, path, &mut ConsoleAsker) {
        Flow::Done(r) => {
            let conv = r.get("conversation").and_then(Value::as_str).map(str::to_string);
            if let Some(c) = &conv {
                println!("대화를 정리했습니다: {}", archive.join(c).display());
            }
            for l in summarize(&r) {
                println!("{l}");
            }
            // 결과 폴더의 안내 파일: 없으면 만들고, README 의 채팅방 목록만 새로 쓴다
            match guide::ensure(&settings.result_dir, &archive) {
                Ok(rep) => {
                    if !rep.created.is_empty() {
                        println!("안내 파일을 만들었습니다: {}", rep.created.join(", "));
                    }
                    if rep.rooms_markers_missing {
                        println!("주의: README.md 에 채팅방 목록 표시(<!-- rooms:begin -->)가 없어 목록을 쓰지 않았습니다.");
                    }
                }
                Err(e) => println!("주의: 안내 파일을 쓰지 못했습니다: {e}"),
            }
            (Outcome::Done, conv)
        }
        Flow::Skipped => {
            println!("정리하지 않았습니다. 내보낸 파일은 그대로 남아 있습니다: {}", path.display());
            (Outcome::Skipped, None)
        }
        Flow::Failed(e) => {
            println!("정리하지 못했습니다 [{}]: {}", e.code, e.message);
            println!("내보낸 파일은 그대로 남아 있습니다. 문제가 계속되면 위 메시지를 알려 주세요.");
            (Outcome::Failed, None)
        }
    }
}

fn archive_dir(s: &Settings) -> PathBuf {
    s.result_dir.join("archive")
}

fn exports_dir(s: &Settings) -> PathBuf {
    s.result_dir.join("exports")
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
        out.push("이미 정리한 내보내기입니다. 달라진 것이 없습니다.".to_string());
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

/// 아카이브에서 서랍에 타일로 보일 메시지(사진·동영상)를 읽는다.
fn collect_media(archive: &Path, conv: &str) -> Vec<MediaRec> {
    let Ok((st, _)) = Archive::new(archive, conv).load() else { return Vec::new() };
    st.registry
        .values()
        .enumerate()
        .filter(|(_, r)| r.kind == "message" && r.status == "active" && (r.content_type == "image" || r.content_type == "video"))
        .map(|(i, r)| {
            let is_video = r.content_type == "video";
            MediaRec {
                date: r.date.clone(),
                hhmm: r.hhmm.clone().unwrap_or_default(),
                order: i,
                is_video,
                count: if is_video { 1 } else { r.image_count as usize },
                unlinked: r.attachment_ids.is_empty(),
            }
        })
        .collect()
}

/// 서랍에서 아직 보관하지 않은 사진(과 동영상)을 저장하고 메시지와 연결한다. 동영상은 설정에 따라 묻는다.
fn save_photos(title: &str, settings: &mut Settings, cfg_path: &Path, conv: &str) {
    let archive = archive_dir(settings);
    let media = collect_media(&archive, conv);
    let (plan, remember) = media_plan::plan(&media, settings.videos, &mut ConsoleAsker);
    if let Some(v) = remember {
        settings.videos = v;
        save_settings(settings, cfg_path);
    }
    let Some(plan) = plan else {
        println!("다운로드할 새 사진·동영상이 없습니다.");
        return;
    };
    println!(
        "\n사진{} {}개(서랍 {}칸)를 다운로드합니다. 마우스와 키보드에서 손을 떼세요. (마우스를 움직이면 일시정지, 중단: Ctrl+D)",
        if plan.keep_videos { "·동영상" } else { "" },
        plan.files,
        plan.tiles
    );
    let dir = settings.download_dir.clone();
    let opt = photos::Options { newest: Some(plan.tiles), expect_files: Some(plan.files), skip_videos: !plan.keep_videos, ..Default::default() };
    match photos::download_photos(title, &dir, &Guard::new(), &opt) {
        Ok(meta) => {
            let saved = meta["saved_files"].as_array().map_or(0, |a| a.len());
            println!("다운로드했습니다: {saved}개 → {}", dir.display());
            for w in meta["warnings"].as_array().into_iter().flatten() {
                println!("주의: {}", w.as_str().unwrap_or_default());
            }
            match ingest_attachments(&Archive::new(&archive, conv), &dir, plan.keep_videos) {
                Ok(r) => {
                    let vids = r["saved_videos"].as_u64().unwrap_or(0);
                    println!(
                        "보관했습니다: 새로 {}개{}, 메시지에 연결 {}개",
                        r["saved"],
                        if vids > 0 { format!(" (동영상 {vids}개 포함)") } else { String::new() },
                        r["linked"]
                    );
                    for g in r["unmatched_groups"].as_array().into_iter().flatten() {
                        let (kind, n) = if g.get("video_messages").is_some() { ("동영상", &g["video_messages"]) } else { ("사진", &g["image_messages"]) };
                        println!(
                            "주의: {} 에 {kind} 메시지 {n}개, 파일 {}개라서 연결하지 않았습니다 (추측하지 않습니다)",
                            g["minute"].as_str().unwrap_or_default(),
                            g["files"]
                        );
                    }
                }
                Err(e) => println!("연결하지 못했습니다 [{}]: {}", e.code, e.message),
            }
        }
        Err(WinError::Aborted) => println!("\nCtrl+D 로 중단했습니다. 다운로드한 파일은 보관하지 않았습니다."),
        Err(e) => println!("\n다운로드하지 못했습니다: {e}"),
    }
}

fn collect_and_ingest(title: &str, settings: &mut Settings, cfg_path: &Path) -> Outcome {
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
    println!("\n내보내기를 수집했습니다: {}", path.display());
    for w in meta["warnings"].as_array().into_iter().flatten() {
        println!("주의: {}", w.as_str().unwrap_or_default());
    }
    let (outcome, conv) = ingest_and_summarize(&path, settings);
    if let Some(c) = conv {
        save_photos(title, settings, cfg_path, &c);
    }
    outcome
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
    println!("\n정리합니다: {}", path.display());
    let _ = ingest_and_summarize(path, settings);
}

/// 사용자가 입력한 한 줄의 뜻. 방 번호와 명령은 한 번에 하나만 쓸 수 있다.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Rooms(Vec<usize>),
    OpenFolder,
    Settings,
    Quit,
    /// 아무것도 입력하지 않았다 (목록을 다시 불러온다)
    Empty,
    /// 알 수 없거나 섞어 쓴 입력. 안내 문구를 담는다.
    Invalid(String),
}

fn mixed_input_message(t: &str) -> String {
    format!("방 번호와 명령은 함께 쓸 수 없습니다. 하나만 입력하세요. (입력: {t})")
}

/// 입력 한 줄을 해석한다. `n_rooms` 는 방 번호의 범위.
pub fn parse_command(line: &str, n_rooms: usize) -> Command {
    let t = line.trim();
    if t.is_empty() {
        return Command::Empty;
    }
    match t.to_lowercase().as_str() {
        "o" => return Command::OpenFolder,
        "s" => return Command::Settings,
        "q" => return Command::Quit,
        _ => {}
    }
    // `1s`, `s1`, `2q`, `1 o` 처럼 번호와 명령을 섞은 입력
    let lower = t.to_lowercase();
    if lower.chars().any(|c| c.is_ascii_digit()) && lower.chars().any(|c| matches!(c, 'o' | 's' | 'q')) {
        return Command::Invalid(mixed_input_message(t));
    }
    match parse_selection(t, n_rooms) {
        Ok(v) => Command::Rooms(v),
        Err(msg) => Command::Invalid(msg),
    }
}

const COL: usize = 4; // 항목 이름과 값 사이의 공백

/// 항목 이름을 칸 수 기준으로 `width` 에 맞춰 오른쪽을 채운다.
fn label(name: &str, width: usize) -> String {
    format!("{name}{}", " ".repeat(width.saturating_sub(screen::display_width(name))))
}

/// 시작 화면 상자 안의 줄들. 경로는 줄마다 완전한 한 줄이라 그대로 복사해서 쓸 수 있다.
pub fn header_rows(cfg_path: &Path, s: &Settings, status: &LoadStatus, notes: &[String]) -> Vec<Row> {
    let mut v = vec![Row::Blank];
    let mut info: Vec<String> = Vec::new();
    match status {
        LoadStatus::Created => info.push("처음 실행이라 기본 설정 파일을 새로 만들었습니다.".to_string()),
        LoadStatus::Migrated(old) => info.push(format!("이전 설정({})을 읽어 새 형식으로 옮겼습니다. 옛 파일은 그대로 둡니다.", old.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())),
        LoadStatus::Invalid(why) => info.push(format!("주의: 설정 파일을 읽지 못해 이번에는 기본값을 씁니다 ({why}). 파일은 덮어쓰지 않았습니다.")),
        LoadStatus::Loaded => {}
    }
    info.extend(notes.iter().map(|n| format!("주의: {n}")));
    if !info.is_empty() {
        v.extend(info.into_iter().map(|l| Row::Text(format!("  {l}"))));
        v.push(Row::Blank);
    }
    let names = ["결과 폴더", "다운로드 폴더", "동영상 보관"];
    let width = names.iter().map(|n| screen::display_width(n)).max().unwrap_or(0) + COL;
    let pad = " ".repeat(width);
    v.push(Row::Text(format!("  {}", cfg_path.display())));
    v.push(Row::Rule);
    v.push(Row::Text(format!("  {}{}", label(names[0], width), s.result_dir.display())));
    v.push(Row::Text(format!("  {pad}├─ archive\\    # 정리한 대화")));
    v.push(Row::Text(format!("  {pad}└─ exports\\    # 내보낸 TXT")));
    v.push(Row::Text(format!("  {}{}", label(names[1], width), s.download_dir.display())));
    v.push(Row::Blank);
    v.push(Row::Text("  옵션".to_string()));
    v.push(Row::Text(format!("  {}{}", label(names[2], width), videos_choices(s.videos))));
    v.push(Row::Blank);
    v
}

/// `[물어보기]  항상 보관  보관 안 함` — 지금 값은 대괄호, 나머지는 고를 수 있는 값이다.
pub fn videos_choices(current: settings::Videos) -> String {
    settings::Videos::ALL
        .iter()
        .map(|v| if *v == current { format!("[{}]", v.label()) } else { v.label().to_string() })
        .collect::<Vec<_>>()
        .join("  ")
}

fn print_header(cfg_path: &Path, s: &Settings, status: &LoadStatus, notes: &[String]) {
    let title = format!("kkt-manual-export-v{}", settings::VERSION);
    for l in screen::render(&title, &header_rows(cfg_path, s, status, notes), screen::terminal_width()) {
        println!("{l}");
    }
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
        println!("{}", cfg_path.display());
        println!("  1) 결과 폴더: {}", s.result_dir.display());
        println!("  2) 다운로드 폴더: {}", s.download_dir.display());
        println!("     (카카오톡 설정의 '사진 저장 위치'와 같아야 다운로드한 파일을 확인할 수 있습니다. 이 프로그램은 그 폴더를 바꾸지 않고 지켜보기만 합니다)");
        println!("  3) 동영상 보관: {}  (누를 때마다 바뀝니다)", videos_choices(s.videos));
        println!("  g) 안내 파일(README.md, AGENTS.md) 다시 만들기");
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
                    s.result_dir = p;
                } else {
                    if !p.is_dir() {
                        println!("주의: 아직 없는 폴더입니다. 카카오톡이 사진을 이 폴더에 내려받도록 설정되어 있는지 확인하세요.");
                    }
                    s.download_dir = p;
                }
                save_settings(s, cfg_path);
            }
            "3" => {
                s.videos = s.videos.next();
                save_settings(s, cfg_path);
            }
            "g" => {
                let q = format!(
                    "결과 폴더의 README.md 와 AGENTS.md 를 새 안내문으로 다시 만듭니다.\n  직접 고친 내용은 사라집니다.\n  ({})",
                    s.result_dir.display()
                );
                let opts = vec!["취소 (그대로 둠)".to_string(), "다시 만들기".to_string()];
                if ConsoleAsker.choose(&q, &opts, 0) == 1 {
                    match guide::recreate(&s.result_dir, &archive_dir(s)) {
                        Ok(()) => println!("다시 만들었습니다: README.md, AGENTS.md"),
                        Err(e) => println!("다시 만들지 못했습니다: {e}"),
                    }
                }
            }
            "r" => {
                *s = Settings::default();
                save_settings(s, cfg_path);
            }
            _ => println!("1, 2, 3, g, r 중에서 고르세요."),
        }
    }
}

/// 다음에 무엇을 할지.
enum Next {
    Quit,
    /// 목록을 다시 불러오고 입력을 받는다
    Again,
    /// 설정이나 옵션이 바뀌었을 수 있으니 시작 화면도 다시 그린다
    Redraw,
}

fn print_commands(first: &str) {
    println!("\n아래 중 하나만 입력하세요.");
    if !first.is_empty() {
        println!("  {first}");
    }
    println!("  o         결과 폴더 열기");
    println!("  s         설정");
    println!("  q         종료");
}

fn do_open(s: &Settings) {
    if let Err(e) = open_folder(&s.result_dir) {
        println!("{e}");
    }
}

/// Windows 가 아닐 때: 카카오톡 조작(수집)은 못 하므로 직접 내보낸 TXT 를 골라 정리만 한다.
fn files_menu(s: &mut Settings, cfg_path: &Path) -> Next {
    println!("\n이 컴퓨터에서는 카카오톡을 자동으로 조작할 수 없습니다 (수집은 Windows 전용).");
    println!("카카오톡에서 직접 대화를 내보낸 TXT 파일을 고르면 정리해 드립니다.\n");
    let files = find_exports(&search_dirs(), &s.result_dir, 20);
    if files.is_empty() {
        println!("다운로드·문서·바탕화면에서 내보내기 파일(KakaoTalk…txt)을 찾지 못했습니다.");
    } else {
        println!("찾은 내보내기 파일 (최근 순):");
        for (i, f) in files.iter().enumerate() {
            println!("  {}) {}", i + 1, f.display());
        }
    }
    print_commands("번호 또는 파일 경로   정리를 시작 (파일을 이 창에 끌어다 놓아도 됩니다)");
    loop {
        let Some(line) = prompt("> ") else { return Next::Quit };
        match line.trim().to_lowercase().as_str() {
            "" => return Next::Again,
            "q" => return Next::Quit,
            "o" => {
                do_open(s);
                return Next::Again;
            }
            "s" => {
                settings_menu(s, cfg_path);
                return Next::Redraw;
            }
            _ => {}
        }
        if let Some(i) = parse_choice(&line, files.len()) {
            ingest_and_report(&files[i], s);
            return Next::Again;
        }
        let p = PathBuf::from(clean_path(&line));
        if p.is_file() {
            ingest_and_report(&p, s);
            return Next::Again;
        }
        println!("번호나 올바른 파일 경로가 아닙니다.");
    }
}

/// Windows: 열려 있는 방 목록에서 골라 수집한다.
fn rooms_menu(s: &mut Settings, cfg_path: &Path) -> Next {
    let titles = window::list_chat_titles();
    println!();
    if titles.is_empty() {
        println!("열려 있는 채팅방이 없습니다. 수집할 방을 카카오톡에서 창으로 열어 두세요. (그냥 Enter = 다시 찾기)");
        print_commands("");
    } else {
        println!("열려 있는 채팅방:");
        for (i, t) in titles.iter().enumerate() {
            println!("  {}) {t}", i + 1);
        }
        print_commands("방번호(ex: 1, 3, 4 | 1-3 | a | all)   수집을 시작");
    }
    loop {
        let Some(line) = prompt("> ") else { return Next::Quit };
        match parse_command(&line, titles.len()) {
            Command::Quit => return Next::Quit,
            Command::Empty => return Next::Again,
            Command::OpenFolder => {
                do_open(s);
                return Next::Again;
            }
            Command::Settings => {
                settings_menu(s, cfg_path);
                return Next::Redraw;
            }
            Command::Rooms(picked) if !titles.is_empty() => {
                collect_rooms(&titles, &picked, s, cfg_path);
                return Next::Redraw;
            }
            Command::Rooms(_) => println!("열려 있는 채팅방이 없습니다."),
            Command::Invalid(msg) => println!("{msg}"),
        }
    }
}

/// 고른 방들을 차례로 처리한다. Ctrl+D 로 중단하면 남은 방은 건너뛴다.
fn collect_rooms(titles: &[String], picked: &[usize], s: &mut Settings, cfg_path: &Path) {
    let (mut done, mut failed, mut declined, mut remaining) = (0, 0, 0, 0);
    for (k, &i) in picked.iter().enumerate() {
        if picked.len() > 1 {
            println!("\n===== [{}/{}] {} =====", k + 1, picked.len(), titles[i]);
        }
        match collect_and_ingest(&titles[i], s, cfg_path) {
            Outcome::Done => done += 1,
            Outcome::Skipped => declined += 1,
            Outcome::Failed => failed += 1,
            Outcome::Aborted => {
                remaining = picked.len() - k - 1;
                failed += 1;
                break;
            }
        }
    }
    if picked.len() > 1 {
        println!(
            "\n전체 결과: 완료 {done}개, 정리하지 않음 {declined}개, 실패·중단 {failed}개{}",
            if remaining > 0 { format!(", 건너뜀 {remaining}개 (Ctrl+D 로 중단)") } else { String::new() }
        );
    }
    println!();
}

pub fn run() -> i32 {
    sys::set_console_utf8();
    let cfg_path = settings::config_path();
    let settings::Loaded { settings: mut s, mut status, mut notes } = Settings::load(&cfg_path);
    let mut redraw = true;
    loop {
        if redraw {
            print_header(&cfg_path, &s, &status, &notes);
            // 처음 실행 안내와 읽다가 알게 된 것은 한 번만 보여 준다
            status = LoadStatus::Loaded;
            notes.clear();
        }
        let next = if sys::SUPPORTED { rooms_menu(&mut s, &cfg_path) } else { files_menu(&mut s, &cfg_path) };
        match next {
            Next::Quit => return 0,
            Next::Again => redraw = false,
            Next::Redraw => redraw = true,
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

    fn texts(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| match r { Row::Text(t) => t.clone(), Row::Blank => String::new(), Row::Rule => "---".into() }).collect()
    }

    #[test]
    fn header_rows_follow_the_agreed_layout() {
        let cfg = Path::new(r"C:\Users\spgmb\AppData\Roaming\kkt-manual-export\config.toml");
        let s = Settings { result_dir: PathBuf::from(r"C:\Users\spgmb\Downloads\kkt-manual-export-archive"), download_dir: PathBuf::from(r"C:\Users\spgmb\Documents\카카오톡 받은 파일"), videos: settings::Videos::Ask };
        let t = texts(&header_rows(cfg, &s, &LoadStatus::Loaded, &[]));
        // 맨 위: 빈 줄, 설정 파일 경로(이름표 없이), 구분선, 결과 폴더, 트리, 다운로드 폴더, 빈 줄, 옵션, 동영상 보관, 빈 줄
        assert_eq!(t[0], "");
        assert_eq!(t[1], r"  C:\Users\spgmb\AppData\Roaming\kkt-manual-export\config.toml");
        assert_eq!(t[2], "---");
        assert!(t[3].starts_with("  결과 폴더") && t[3].ends_with(r"C:\Users\spgmb\Downloads\kkt-manual-export-archive"));
        assert!(t[4].contains("├─ archive\\    # 정리한 대화"), "{:?}", t[4]);
        assert!(t[5].contains("└─ exports\\    # 내보낸 TXT"), "{:?}", t[5]);
        assert!(t[6].starts_with("  다운로드 폴더") && t[6].ends_with(r"C:\Users\spgmb\Documents\카카오톡 받은 파일"));
        assert_eq!((t[7].as_str(), t[8].as_str()), ("", "  옵션"));
        assert!(t[9].starts_with("  동영상 보관") && t[9].ends_with("[물어보기]  항상 보관  보관 안 함"), "{:?}", t[9]);
        assert_eq!(t.len(), 11);
        assert!(!t.iter().any(|l| l.contains("설정 파일") || l.contains("저장 폴더")), "옛 용어가 남으면 안 된다");
    }

    #[test]
    fn path_values_start_in_the_same_column_with_four_spaces_after_the_longest_label() {
        let s = Settings::default();
        let t = texts(&header_rows(Path::new("/c/config.toml"), &s, &LoadStatus::Loaded, &[]));
        let col = |line: &str| screen::display_width(&line[..line.find(&s.result_dir.to_string_lossy().to_string()).or_else(|| line.find(&s.download_dir.to_string_lossy().to_string())).unwrap()]);
        assert_eq!(col(&t[3]), col(&t[6]), "결과 폴더와 다운로드 폴더의 경로는 같은 칸에서 시작한다");
        assert_eq!(col(&t[3]), 2 + screen::display_width("다운로드 폴더") + COL);
        // 트리와 옵션 값도 같은 칸
        assert_eq!(screen::display_width(&t[4][..t[4].find('├').unwrap()]), col(&t[3]));
        assert_eq!(screen::display_width(&t[9][..t[9].find('[').unwrap()]), col(&t[3]));
    }

    #[test]
    fn header_notes_show_first_run_migration_and_problems_above_the_paths() {
        let cfg = Path::new("/c/config.toml");
        let s = Settings::default();
        let created = texts(&header_rows(cfg, &s, &LoadStatus::Created, &[]));
        assert_eq!(created[1], "  처음 실행이라 기본 설정 파일을 새로 만들었습니다.");
        assert_eq!(created[2], "");
        assert_eq!(created[3], "  /c/config.toml");
        let migrated = texts(&header_rows(cfg, &s, &LoadStatus::Migrated(PathBuf::from("/c/config.json")), &[]));
        assert!(migrated[1].contains("config.json") && migrated[1].contains("옛 파일은 그대로"));
        let bad = texts(&header_rows(cfg, &s, &LoadStatus::Invalid("denied".into()), &["videos 값 x".to_string()]));
        assert!(bad[1].contains("덮어쓰지 않았습니다") && bad[1].contains("denied"));
        assert_eq!(bad[2], "  주의: videos 값 x");
    }

    #[test]
    fn rendered_header_box_is_aligned_and_titled_with_the_version() {
        let s = Settings::default();
        let lines = screen::render(&format!("kkt-manual-export-v{}", settings::VERSION), &header_rows(Path::new("/c/config.toml"), &s, &LoadStatus::Loaded, &[]), Some(500));
        let w: Vec<usize> = lines.iter().map(|l| screen::display_width(l)).collect();
        assert!(w.iter().all(|x| *x == w[0]), "{w:?}\n{}", lines.join("\n"));
        assert!(lines[0].contains(&format!("kkt-manual-export-v{}", settings::VERSION)));
    }

    #[test]
    fn videos_choices_mark_the_current_value() {
        assert_eq!(videos_choices(settings::Videos::Ask), "[물어보기]  항상 보관  보관 안 함");
        assert_eq!(videos_choices(settings::Videos::Keep), "물어보기  [항상 보관]  보관 안 함");
        assert_eq!(videos_choices(settings::Videos::Skip), "물어보기  항상 보관  [보관 안 함]");
    }

    #[test]
    fn one_input_means_one_thing() {
        assert_eq!(parse_command("o", 2), Command::OpenFolder);
        assert_eq!(parse_command(" S ", 2), Command::Settings);
        assert_eq!(parse_command("q", 2), Command::Quit);
        assert_eq!(parse_command("", 2), Command::Empty);
        assert_eq!(parse_command("   ", 2), Command::Empty);
        assert_eq!(parse_command("1, 2", 2), Command::Rooms(vec![0, 1]));
        assert_eq!(parse_command("all", 2), Command::Rooms(vec![0, 1]));
        assert_eq!(parse_command("A", 3), Command::Rooms(vec![0, 1, 2]));
        assert_eq!(parse_command("1-2", 3), Command::Rooms(vec![0, 1]));
    }

    #[test]
    fn numbers_and_commands_cannot_be_combined() {
        for mixed in ["1s", "2q", "s1", "q 2", "1 o", "1,2s", "o1"] {
            match parse_command(mixed, 3) {
                Command::Invalid(m) => assert!(m.contains("함께 쓸 수 없습니다") && m.contains(mixed.trim()), "{mixed}: {m}"),
                other => panic!("{mixed}: {other:?}"),
            }
        }
        assert!(matches!(parse_command("x", 3), Command::Invalid(m) if m.contains("잘못된 입력")));
        assert!(matches!(parse_command("9", 3), Command::Invalid(m) if m.contains("1 ~ 3")));
        assert!(matches!(parse_command("ss", 3), Command::Invalid(_)), "명령은 한 글자만");
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
        assert!(s[0].contains("이미 정리한"));
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
