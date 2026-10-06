//! 결과 폴더의 안내 파일 `README.md`, `AGENTS.md`.
//!
//! 안내문 원본은 `templates/` 에 있고 실행 파일에 들어 있다. 정리를 마칠 때마다 다음을 한다.
//! - 없으면 만든다.
//! - 있으면 `README.md` 의 `<!-- rooms:begin -->` ~ `<!-- rooms:end -->` 사이(채팅방 목록)만 새로 쓴다.
//!   그 밖에 사용자가 고친 글은 건드리지 않고, `AGENTS.md` 는 만든 뒤 다시 쓰지 않는다.
//! 전체를 새 안내문으로 바꾸는 것은 사용자가 설정에서 요청할 때뿐이다 (`recreate`).

use kkt_core::state;
use std::fs;
use std::io;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

const README_TEMPLATE: &str = include_str!("../templates/result-README.md");
const AGENTS_TEMPLATE: &str = include_str!("../templates/result-AGENTS.md");
pub const BEGIN: &str = "<!-- rooms:begin -->";
pub const END: &str = "<!-- rooms:end -->";

/// 채팅방 목록의 한 행.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomRow {
    pub title: String,
    pub folder: String,
    pub messages: usize,
    /// `YYYY-MM-DD HH:MM`, 알 수 없으면 `-`
    pub last: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GuideReport {
    /// 이번에 새로 만든 파일
    pub created: Vec<&'static str>,
    /// 방 목록 구역이 바뀌었다
    pub rooms_updated: bool,
    /// README.md 에 방 목록 표시(`rooms:begin/end`)가 없어서 목록을 쓰지 않았다
    pub rooms_markers_missing: bool,
}

fn with_sep(text: &str) -> String {
    text.replace("{SEP}", &MAIN_SEPARATOR.to_string())
}

/// 표 안의 글자로 쓸 수 있게 줄바꿈을 없애고 `|` 를 이스케이프한다.
fn cell(s: &str) -> String {
    s.replace(['\r', '\n'], " ").replace('|', "\\|").trim().to_string()
}

/// `2026-10-02T10:18:07+09:00` → `2026-10-02 10:18`
fn short_time(saved_at: &str) -> String {
    let t: String = saved_at.chars().take(16).collect();
    t.replace('T', " ")
}

/// 아카이브 폴더의 방들을 읽는다. 마지막 정리가 최근인 방이 앞에 온다.
pub fn list_rooms(archive: &Path) -> Vec<RoomRow> {
    let Ok(rd) = fs::read_dir(archive) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("events.jsonl").exists()).collect();
    dirs.sort();
    let mut rows: Vec<RoomRow> = dirs
        .into_iter()
        .filter_map(|d| {
            let (st, _) = state::load(&d.join("events.jsonl")).ok()?;
            let id = d.file_name()?.to_string_lossy().to_string();
            Some(RoomRow {
                title: st.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "(제목 없음)".to_string()),
                folder: format!("archive{MAIN_SEPARATOR}{id}"),
                messages: st.registry.values().filter(|r| r.kind == "message").count(),
                last: st.ingests.last().and_then(|i| i.saved_at.as_deref()).map(short_time).unwrap_or_else(|| "-".to_string()),
            })
        })
        .collect();
    rows.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.folder.cmp(&b.folder)));
    rows
}

/// 방 목록 표 (구역 안에 들어갈 줄들).
pub fn rooms_table(rows: &[RoomRow]) -> String {
    if rows.is_empty() {
        return "(아직 정리한 채팅방이 없습니다)".to_string();
    }
    let mut t = String::from("| 채팅방 | 폴더 | 메시지 | 마지막 정리 |\n|---|---|---|---|");
    for r in rows {
        t.push_str(&format!("\n| {} | `{}` | {} | {} |", cell(&r.title), r.folder, r.messages, r.last));
    }
    t
}

/// 본문에서 방 목록 구역을 `table` 로 바꾼다. 표시가 없으면 `None`. 줄바꿈 방식(LF/CRLF)은 본문을 따른다.
pub fn replace_rooms(text: &str, table: &str) -> Option<String> {
    let b = text.find(BEGIN)?;
    let e = b + text[b..].find(END)?;
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body = table.replace('\n', nl);
    Some(format!("{}{BEGIN}{nl}{body}{nl}{}", &text[..b], &text[e..]))
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("md.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// 새 README 본문 (방 목록 포함).
fn fresh_readme(rows: &[RoomRow]) -> String {
    replace_rooms(&with_sep(README_TEMPLATE), &rooms_table(rows)).expect("템플릿에는 방 목록 표시가 있다")
}

/// 정리를 마칠 때마다 부른다. 없는 안내 파일은 만들고, README 의 방 목록만 갱신한다.
pub fn ensure(result_dir: &Path, archive: &Path) -> io::Result<GuideReport> {
    fs::create_dir_all(result_dir)?;
    let rows = list_rooms(archive);
    let mut rep = GuideReport::default();

    let readme = result_dir.join("README.md");
    if readme.exists() {
        let old = fs::read_to_string(&readme)?;
        match replace_rooms(&old, &rooms_table(&rows)) {
            Some(new) if new != old => {
                write_atomic(&readme, &new)?;
                rep.rooms_updated = true;
            }
            Some(_) => {}
            None => rep.rooms_markers_missing = true,
        }
    } else {
        write_atomic(&readme, &fresh_readme(&rows))?;
        rep.created.push("README.md");
    }

    let agents = result_dir.join("AGENTS.md");
    if !agents.exists() {
        write_atomic(&agents, &with_sep(AGENTS_TEMPLATE))?;
        rep.created.push("AGENTS.md");
    }
    Ok(rep)
}

/// 두 파일을 새 안내문으로 다시 만든다 (직접 고친 내용은 사라진다. 호출하기 전에 사용자에게 확인한다).
pub fn recreate(result_dir: &Path, archive: &Path) -> io::Result<()> {
    fs::create_dir_all(result_dir)?;
    write_atomic(&result_dir.join("README.md"), &fresh_readme(&list_rooms(archive)))?;
    write_atomic(&result_dir.join("AGENTS.md"), &with_sep(AGENTS_TEMPLATE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest_flow::{ingest_interactive, Asker, Flow};

    struct Defaults;
    impl Asker for Defaults {
        fn choose(&mut self, _q: &str, _o: &[String], default: usize) -> usize {
            default
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kkt-guide-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// 합성 내보내기를 만들어 아카이브에 정리한다.
    fn add_room(work: &Path, archive: &Path, title: &str, saved: &str, lines: &[&str]) {
        let mut t = format!("{title} 님과 카카오톡 대화\n저장한 날짜 : {saved}\n\n--------------- 2026년 10월 2일 금요일 ---------------\n");
        for l in lines {
            t.push_str(l);
            t.push('\n');
        }
        let p = work.join(format!("{}.txt", saved.replace([' ', ':'], "_")));
        fs::write(&p, t.replace('\n', "\r\n")).unwrap();
        assert!(matches!(ingest_interactive(archive, &p, &mut Defaults), Flow::Done(_)));
    }

    #[test]
    fn table_has_one_row_per_room_and_escapes_pipes() {
        let rows = vec![
            RoomRow { title: "a|b".into(), folder: "archive/kt_1".into(), messages: 3, last: "2026-10-02 10:00".into() },
            RoomRow { title: "둘\n째".into(), folder: "archive/kt_2".into(), messages: 0, last: "-".into() },
        ];
        let t = rooms_table(&rows);
        assert!(t.starts_with("| 채팅방 | 폴더 | 메시지 | 마지막 정리 |\n|---|---|---|---|\n"));
        assert!(t.contains("| a\\|b | `archive/kt_1` | 3 | 2026-10-02 10:00 |"), "{t}");
        assert!(t.contains("| 둘 째 | `archive/kt_2` | 0 | - |"), "{t}");
        assert_eq!(rooms_table(&[]), "(아직 정리한 채팅방이 없습니다)");
    }

    #[test]
    fn only_the_rooms_section_is_replaced() {
        let text = format!("앞 글\n{BEGIN}\n옛 표\n{END}\n뒤 글\n");
        let out = replace_rooms(&text, "새 표").unwrap();
        assert_eq!(out, format!("앞 글\n{BEGIN}\n새 표\n{END}\n뒤 글\n"));
        assert_eq!(replace_rooms(&out, "새 표").unwrap(), out, "같은 표를 다시 쓰면 그대로");
        assert!(replace_rooms("표시 없음", "x").is_none());
        assert!(replace_rooms(&format!("{END}\n{BEGIN}"), "x").is_none(), "순서가 뒤집히면 쓰지 않는다");
        let crlf = replace_rooms(&format!("a\r\n{BEGIN}\r\n{END}\r\n"), "1\n2").unwrap();
        assert_eq!(crlf, format!("a\r\n{BEGIN}\r\n1\r\n2\r\n{END}\r\n"), "CRLF 파일은 CRLF 로");
    }

    #[test]
    fn creates_both_files_with_the_room_name_and_never_rewrites_agents() {
        let (w, base) = (tmp("w1"), tmp("base1"));
        let archive = base.join("archive");
        add_room(&w, &archive, "시험 방", "2026-10-02 10:00:00", &["[민수] [오전 9:00] 하나", "[지영] [오전 9:01] 둘"]);
        let rep = ensure(&base, &archive).unwrap();
        assert_eq!(rep.created, ["README.md", "AGENTS.md"]);
        let readme = fs::read_to_string(base.join("README.md")).unwrap();
        let sep = MAIN_SEPARATOR;
        assert!(readme.contains("| 시험 방 | `archive") && readme.contains("| 2 | 2026-10-02 10:00 |"), "{readme}");
        assert!(readme.contains(&format!("`exports{sep}`")) && !readme.contains("{SEP}"));
        let agents = fs::read_to_string(base.join("AGENTS.md")).unwrap();
        assert!(agents.contains(&format!("archive{sep}<폴더>{sep}events.jsonl")) && !agents.contains("{SEP}"));
        // 사용자가 AGENTS.md 를 고치면 그대로 둔다
        fs::write(base.join("AGENTS.md"), "내가 고침").unwrap();
        let rep2 = ensure(&base, &archive).unwrap();
        assert!(rep2.created.is_empty() && !rep2.rooms_updated);
        assert_eq!(fs::read_to_string(base.join("AGENTS.md")).unwrap(), "내가 고침");
        let _ = (fs::remove_dir_all(&w), fs::remove_dir_all(&base));
    }

    #[test]
    fn new_rooms_are_added_and_user_notes_outside_the_section_survive() {
        let (w, base) = (tmp("w2"), tmp("base2"));
        let archive = base.join("archive");
        add_room(&w, &archive, "첫 방", "2026-10-02 10:00:00", &["[민수] [오전 9:00] 하나", "[지영] [오전 9:01] 둘"]);
        ensure(&base, &archive).unwrap();
        // 사용자가 구역 밖에 메모를 적는다
        let mut text = fs::read_to_string(base.join("README.md")).unwrap();
        text.push_str("\n## 내 메모\n- 백업은 매주 금요일\n");
        fs::write(base.join("README.md"), &text).unwrap();
        add_room(&w, &archive, "둘째 방", "2026-10-03 11:30:00", &["[가] [오전 7:00] 다른", "[나] [오전 7:01] 이야기", "[가] [오전 7:02] 완전히", "[나] [오전 7:03] 달라요"]);
        let rep = ensure(&base, &archive).unwrap();
        assert!(rep.rooms_updated && rep.created.is_empty());
        let after = fs::read_to_string(base.join("README.md")).unwrap();
        assert!(after.contains("| 첫 방 |") && after.contains("| 둘째 방 |"));
        assert!(after.contains("## 내 메모\n- 백업은 매주 금요일"), "구역 밖의 글은 그대로");
        assert!(after.find("둘째 방").unwrap() < after.find("첫 방").unwrap(), "최근에 정리한 방이 앞에 온다");
        // 아무것도 안 바뀌면 파일을 다시 쓰지 않는다
        let m1 = fs::metadata(base.join("README.md")).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(!ensure(&base, &archive).unwrap().rooms_updated);
        assert_eq!(fs::metadata(base.join("README.md")).unwrap().modified().unwrap(), m1);
        let _ = (fs::remove_dir_all(&w), fs::remove_dir_all(&base));
    }

    #[test]
    fn a_renamed_room_shows_its_latest_title() {
        let (w, base) = (tmp("w3"), tmp("base3"));
        let archive = base.join("archive");
        let lines = ["[민수] [오전 9:00] 하나", "[지영] [오전 9:01] 둘", "[민수] [오전 9:02] 셋", "[지영] [오전 9:03] 넷"];
        add_room(&w, &archive, "옛 이름", "2026-10-02 10:00:00", &lines);
        add_room(&w, &archive, "새 이름", "2026-10-02 10:10:00", &lines);
        let rows = list_rooms(&archive);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "새 이름");
        let _ = (fs::remove_dir_all(&w), fs::remove_dir_all(&base));
    }

    #[test]
    fn missing_markers_mean_the_user_took_over_the_file() {
        let (w, base) = (tmp("w4"), tmp("base4"));
        let archive = base.join("archive");
        add_room(&w, &archive, "방", "2026-10-02 10:00:00", &["[민수] [오전 9:00] 하나", "[지영] [오전 9:01] 둘"]);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("README.md"), "# 내가 새로 쓴 README\n").unwrap();
        let rep = ensure(&base, &archive).unwrap();
        assert!(rep.rooms_markers_missing && !rep.rooms_updated);
        assert_eq!(fs::read_to_string(base.join("README.md")).unwrap(), "# 내가 새로 쓴 README\n", "표시가 없으면 건드리지 않는다");
        let _ = (fs::remove_dir_all(&w), fs::remove_dir_all(&base));
    }

    #[test]
    fn recreate_replaces_both_files_with_the_templates() {
        let (w, base) = (tmp("w5"), tmp("base5"));
        let archive = base.join("archive");
        add_room(&w, &archive, "방", "2026-10-02 10:00:00", &["[민수] [오전 9:00] 하나", "[지영] [오전 9:01] 둘"]);
        fs::write(base.join("README.md"), "고친 글").unwrap();
        fs::write(base.join("AGENTS.md"), "고친 글").unwrap();
        recreate(&base, &archive).unwrap();
        assert!(fs::read_to_string(base.join("README.md")).unwrap().contains("# 카카오톡 대화 보관 폴더"));
        assert!(fs::read_to_string(base.join("AGENTS.md")).unwrap().starts_with("# AGENTS.md"));
        assert!(fs::read_to_string(base.join("README.md")).unwrap().contains("| 방 |"));
        let _ = (fs::remove_dir_all(&w), fs::remove_dir_all(&base));
    }

    #[test]
    fn works_before_any_room_exists() {
        let base = tmp("base6");
        let rep = ensure(&base, &base.join("archive")).unwrap();
        assert_eq!(rep.created, ["README.md", "AGENTS.md"]);
        assert!(fs::read_to_string(base.join("README.md")).unwrap().contains("(아직 정리한 채팅방이 없습니다)"));
        let _ = fs::remove_dir_all(&base);
    }
}
