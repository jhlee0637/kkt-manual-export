//! 인자 없이 실행(더블클릭)하면 뜨는 안내 마당. 열려 있는 카카오톡 방을 고르면 수집부터 정리까지 한 번에 한다.
//!
//! 저장 위치는 `다운로드\kkt-manual-export-archive` (`archive/` 에 정리 결과, `exports/` 에 내보내기 TXT).

use kkt_win::{collect, guard::Guard, sys, window, WinError};
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
            if let Some(c) = r.get("conversation").and_then(Value::as_str) {
                println!("정리 위치: {}", archive.join(c).display());
            }
            for l in summarize(&r) {
                println!("{l}");
            }
        }
        Err(e) => {
            println!("정리하지 못했습니다 [{}]: {}", e.code, e.message);
            println!("내보낸 파일은 그대로 남아 있습니다. 문제가 계속되면 위 메시지를 알려 주세요.");
        }
    }
}

pub fn run() -> i32 {
    sys::set_console_utf8();
    let base = base_dir();
    println!("== 카카오톡 대화 수집 ==");
    println!("저장 위치: {}\n", base.display());
    if !sys::SUPPORTED {
        println!("이 안내 마당은 Windows 에서만 동작합니다. 명령줄 사용법은 `kkt --help` 를 보세요.");
        return 1;
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
