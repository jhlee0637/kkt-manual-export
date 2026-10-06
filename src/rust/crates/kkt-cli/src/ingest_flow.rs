//! 내보내기 TXT 를 아카이브에 반영하되, 사람이 정해야 할 때는 묻는다.
//!
//! 실제로 반영하기 전에 임시 복사본에서 먼저 돌려 본다 (미리보기). 그래야 오류와 경고를 보고 물은 뒤,
//! 답을 받아 한 번만 반영한다. 묻는 것은 세 가지다.
//! - 이 PC 의 기록이 크게 줄었을 때 (`mass_loss`): 기본은 반영하지 않는다.
//! - 어느 방의 기록인지 모호할 때 (`room_title_ambiguous`, `room_rename_ambiguous`): 기존 방에 잇기 / 새 방 / 건너뛰기.
//! - 참가자 이름 변경이 의심되지만 근거가 약할 때: 같은 사람인지.
//! 사용자가 답하지 않거나 입력이 끊기면 항상 가장 안전한 쪽(반영하지 않기, 보류)으로 한다.

use indexmap::IndexMap;
use kkt_core::archive::{decode_export, Archive};
use kkt_core::error::KktError;
use kkt_core::link::{new_conversation_id, overlap, resolve};
use kkt_core::parse::{parse_export, ParsedExport};
use kkt_core::state;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// 선택지를 보여 주고 고른 번호(0부터)를 돌려준다. 입력이 없으면 `default`.
pub trait Asker {
    fn choose(&mut self, question: &str, options: &[String], default: usize) -> usize;
}

#[derive(Debug)]
pub enum Flow {
    /// 반영했다. 결과 JSON 에 `conversation`, `link`, `events`, `warnings` 가 있다.
    Done(Value),
    /// 사용자가 반영하지 않기로 했다.
    Skipped,
    Failed(KktError),
}

/// 기존 방 후보: 폴더 이름(방 ID), 제목, 새 내보내기와 겹치는 메시지 수.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomCandidate {
    pub id: String,
    pub title: Option<String>,
    pub matched: usize,
    pub compared: usize,
}

pub fn list_rooms(archive: &Path, parsed: &ParsedExport) -> Vec<RoomCandidate> {
    let Ok(rd) = fs::read_dir(archive) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("events.jsonl").exists()).collect();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|d| {
            let (st, _) = state::load(&d.join("events.jsonl")).ok()?;
            let (matched, compared) = overlap(&st, parsed);
            Some(RoomCandidate { id: d.file_name()?.to_string_lossy().to_string(), title: st.title.clone(), matched, compared })
        })
        .collect()
}

/// 코어의 오류 문구에서 명령줄 옵션을 안내하는 문장(`--force`, `--conversation` 이 든 문장)을 뺀다.
/// 마법사 사용자에게는 옵션이 아니라 선택지를 보여 주기 때문이다.
pub fn plain_message(msg: &str) -> String {
    let kept: Vec<&str> = msg.trim_end_matches('.').split(". ").filter(|s| !s.contains("--")).collect();
    let mut out = kept.join(". ");
    if !out.ends_with('.') && !out.is_empty() {
        out.push('.');
    }
    out
}

/// 경고 문장에서 이름 변경 의심을 뽑는다: `(옛 이름, 새 이름, 이유)`.
/// 문구는 reconcile 이 만든다: `발신자 이름 변경 의심 'A' -> 'B' (근거 N건)을 확정하지 않았다: {이유}. 맞다면 --accept-rename 'A=B' 로 확인하라`.
pub fn extract_suspects(warnings: &Value) -> Vec<(String, String, String)> {
    const KEY: &str = "--accept-rename '";
    const END: &str = "' 로 확인하라";
    let mut out = Vec::new();
    for w in warnings.as_array().into_iter().flatten().filter_map(Value::as_str) {
        let Some(a) = w.find(KEY) else { continue };
        let rest = &w[a + KEY.len()..];
        let Some(b) = rest.rfind(END) else { continue };
        let Some((old, new)) = rest[..b].split_once('=') else { continue };
        if old.is_empty() || new.is_empty() || new == "None" {
            continue;
        }
        let why = w
            .split_once("을 확정하지 않았다: ")
            .and_then(|(_, r)| r.split_once(". 맞다면"))
            .map(|(why, _)| why.to_string())
            .unwrap_or_default();
        out.push((old.to_string(), new.to_string(), why));
    }
    out
}

/// 임시 복사본에서 반영해 본다. 실제 아카이브는 건드리지 않는다.
fn preview(archive: &Path, conv: &str, path: &Path, parsed: &ParsedExport, force: bool, accept: &IndexMap<String, String>, link: &Value) -> Result<Value, KktError> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!("kkt-preview-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let _ = fs::remove_dir_all(&root);
    let src = archive.join(conv).join("events.jsonl");
    if src.exists() {
        fs::create_dir_all(root.join(conv))?;
        fs::copy(&src, root.join(conv).join("events.jsonl"))?; // 반영은 이벤트 로그만 읽는다
    }
    let r = Archive::new(&root, conv).ingest(path, Some(parsed), force, accept, Some(link));
    let _ = fs::remove_dir_all(&root);
    r
}

fn ask_room(archive: &Path, parsed: &ParsedExport, err: &KktError, ask: &mut dyn Asker) -> Result<Option<String>, KktError> {
    // 반환: Some(기존 방 ID), None 이면 새 방. 건너뛰기는 Err 로 표현하지 않고 호출한 쪽에서 Skipped 로 처리한다.
    let rooms = list_rooms(archive, parsed);
    let title = parsed.title.clone().unwrap_or_default();
    let mut options: Vec<String> = rooms
        .iter()
        .map(|r| {
            format!(
                "'{}' 방에 이어 붙이기 (겹치는 메시지 {}/{}, 폴더 {})",
                r.title.clone().unwrap_or_else(|| "(제목 없음)".into()),
                r.matched,
                r.compared,
                r.id
            )
        })
        .collect();
    options.push("새 방으로 따로 저장하기".to_string());
    options.push("반영하지 않기".to_string());
    let skip = options.len() - 1;
    let q = format!(
        "이 내보내기(제목 '{title}')를 어느 방의 기록에 이어 붙일지 정하지 못했습니다.\n  이유: {}\n  잘못 합치면 되돌리기 어렵습니다. 확실하지 않으면 '반영하지 않기'를 고르세요.",
        plain_message(&err.message)
    );
    let k = ask.choose(&q, &options, skip).min(skip);
    if k == skip {
        return Err(KktError::new("skipped_by_user", "사용자가 반영하지 않기로 했다"));
    }
    Ok(rooms.get(k).map(|r| r.id.clone()))
}

/// 내보내기 하나를 반영한다. 사람이 정해야 할 때는 `ask` 로 묻는다.
pub fn ingest_interactive(archive: &Path, path: &Path, ask: &mut dyn Asker) -> Flow {
    let parsed = match fs::read(path).map_err(KktError::from).and_then(|d| decode_export(&d)).map(|t| parse_export(&t)) {
        Ok(p) => p,
        Err(e) => return Flow::Failed(e),
    };
    if parsed.entries.is_empty() {
        // 읽은 항목이 하나도 없는 파일(빈 파일, 형식이 다른 파일)로 빈 방을 만들지 않는다
        return Flow::Failed(KktError::new("empty_export", "내보내기에서 읽은 항목이 없다. 빈 파일이거나 형식이 바뀌었을 수 있다."));
    }

    // 1) 어느 방인가
    let resolved = match resolve(archive, &parsed, None, false) {
        Ok(r) => Ok(r),
        Err(e) if e.code == "room_title_ambiguous" || e.code == "room_rename_ambiguous" => match ask_room(archive, &parsed, &e, ask) {
            Ok(Some(id)) => resolve(archive, &parsed, Some(&id), true), // 사람이 정했으므로 겹침이 적어도 따른다
            Ok(None) => resolve(archive, &parsed, Some(&new_conversation_id(&parsed)), false),
            Err(skip) if skip.code == "skipped_by_user" => return Flow::Skipped,
            Err(other) => Err(other),
        },
        Err(e) => Err(e),
    };
    let (conv, link) = match resolved {
        Ok(r) => r,
        Err(e) => return Flow::Failed(e),
    };

    // 2) 미리 돌려 보고 물을 것을 모은다
    let mut force = false;
    let mut accept: IndexMap<String, String> = IndexMap::new();
    loop {
        match preview(archive, &conv, path, &parsed, force, &accept, &link) {
            Ok(r) => {
                for (old, new, why) in extract_suspects(&r["warnings"]) {
                    let q = format!(
                        "'{old}' 님이 '{new}' 님으로 이름을 바꾼 같은 사람입니까?\n  확정하지 못한 이유: {}\n  모르면 '보류'를 고르세요. 보류하면 다른 사람으로 기록되고, 나중에 직접 연결할 수 있습니다.",
                        if why.is_empty() { "근거가 부족합니다".to_string() } else { why }
                    );
                    let opts = vec!["같은 사람입니다 (이름 변경으로 기록)".to_string(), "모르겠습니다 / 다른 사람입니다 (보류)".to_string()];
                    if ask.choose(&q, &opts, 1) == 0 {
                        accept.insert(old, new);
                    }
                }
                break;
            }
            Err(e) if e.code == "mass_loss" && !force => {
                let q = format!(
                    "이 PC 의 카카오톡에 보이는 대화가 기존 기록보다 크게 줄었습니다.\n  {}\n  보통 이 PC 의 기록이 지워졌거나(예: QR 1회용 로그인) 내보내기 범위가 달라진 경우입니다.\n  그대로 반영하면 기존 메시지가 '사라짐'으로 표시됩니다 (보관한 기록 자체는 지워지지 않습니다).",
                    plain_message(&e.message)
                );
                let opts = vec!["반영하지 않기 (권장)".to_string(), "그래도 반영하기".to_string()];
                if ask.choose(&q, &opts, 0) == 1 {
                    force = true;
                } else {
                    return Flow::Skipped;
                }
            }
            Err(e) => return Flow::Failed(e),
        }
    }

    // 3) 답을 반영해 실제로 한 번 반영한다
    match Archive::new(archive, &conv).ingest(path, Some(&parsed), force, &accept, Some(&link)) {
        Ok(r) => {
            let mut m = Map::new();
            m.insert("conversation".into(), conv.into());
            m.insert("link".into(), link["status"].clone());
            if let Value::Object(rm) = r {
                m.extend(rm);
            }
            Flow::Done(Value::Object(m))
        }
        Err(e) => Flow::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;

    /// 미리 정해 둔 답을 차례로 돌려주고, 받은 질문을 기록한다.
    struct Script {
        answers: VecDeque<usize>,
        asked: Vec<String>,
    }

    impl Script {
        fn new(a: &[usize]) -> Script {
            Script { answers: a.iter().copied().collect(), asked: Vec::new() }
        }
    }

    impl Asker for Script {
        fn choose(&mut self, q: &str, options: &[String], default: usize) -> usize {
            self.asked.push(format!("{q} | {}", options.join(" / ")));
            self.answers.pop_front().unwrap_or(default)
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kkt-flow-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn export(dir: &Path, name: &str, title: &str, saved: &str, lines: &[&str]) -> PathBuf {
        export_on(dir, name, title, saved, "2026년 10월 2일 금요일", lines)
    }

    fn export_on(dir: &Path, name: &str, title: &str, saved: &str, day: &str, lines: &[&str]) -> PathBuf {
        let mut t = format!("{title} 님과 카카오톡 대화\n저장한 날짜 : {saved}\n\n--------------- {day} ---------------\n");
        for l in lines {
            t.push_str(l);
            t.push_str("\r\n");
        }
        let p = dir.join(name);
        fs::write(&p, t.replace('\n', "\r\n")).unwrap();
        p
    }

    fn log_len(archive: &Path, conv: &str) -> usize {
        fs::read_to_string(archive.join(conv).join("events.jsonl")).map(|s| s.lines().count()).unwrap_or(0)
    }

    const SIX: [&str; 6] = [
        "[민수] [오전 9:00] 하나", "[지영] [오전 9:01] 둘", "[민수] [오전 9:02] 셋", "[지영] [오전 9:03] 넷", "[민수] [오전 9:04] 다섯", "[지영] [오전 9:05] 여섯",
    ];

    fn first_ingest(dir: &Path, archive: &Path) -> String {
        let e1 = export(dir, "e1.txt", "방", "2026-10-02 10:00:00", &SIX);
        match ingest_interactive(archive, &e1, &mut Script::new(&[])) {
            Flow::Done(r) => r["conversation"].as_str().unwrap().to_string(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn normal_ingest_asks_nothing() {
        let (d, a) = (tmp("normal"), tmp("normal-arch"));
        let conv = first_ingest(&d, &a);
        let e2 = export(&d, "e2.txt", "방", "2026-10-02 10:10:00", &[SIX[0], SIX[1], SIX[2], SIX[3], SIX[4], SIX[5], "[민수] [오전 9:06] 일곱"]);
        let mut ask = Script::new(&[]);
        let Flow::Done(r) = ingest_interactive(&a, &e2, &mut ask) else { panic!() };
        assert_eq!(r["conversation"], conv.as_str());
        assert!(ask.asked.is_empty());
        let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
    }

    #[test]
    fn mass_loss_default_is_not_to_ingest_and_leaves_archive_untouched() {
        let (d, a) = (tmp("loss"), tmp("loss-arch"));
        let conv = first_ingest(&d, &a);
        let before = log_len(&a, &conv);
        // 이 PC 에는 오늘 것만 남은 상황: 겹치는 날짜가 없어서 같은 방으로 보이고, 기존 메시지가 모두 사라진 것처럼 보인다
        let e2 = export_on(&d, "e2.txt", "방", "2026-10-06 08:11:32", "2026년 10월 6일 화요일", &["[민수] [오전 8:07] 사진", "[민수] [오전 8:08] 새 글"]);
        let mut ask = Script::new(&[]); // 답이 없으면 기본값
        assert!(matches!(ingest_interactive(&a, &e2, &mut ask), Flow::Skipped));
        assert_eq!(ask.asked.len(), 1);
        assert!(ask.asked[0].contains("크게 줄었습니다") && ask.asked[0].contains("반영하지 않기 (권장)"));
        assert_eq!(log_len(&a, &conv), before, "미리보기는 실제 아카이브를 바꾸지 않는다");
        assert!(!a.join(&conv).join("raw").join("e2.txt").exists());
        let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
    }

    #[test]
    fn mass_loss_can_be_forced_when_the_user_says_so() {
        let (d, a) = (tmp("force"), tmp("force-arch"));
        let conv = first_ingest(&d, &a);
        let before = log_len(&a, &conv);
        let e2 = export_on(&d, "e2.txt", "방", "2026-10-06 08:11:32", "2026년 10월 6일 화요일", &["[민수] [오전 8:07] 사진", "[민수] [오전 8:08] 새 글"]);
        let Flow::Done(r) = ingest_interactive(&a, &e2, &mut Script::new(&[1])) else { panic!() };
        assert!(r["events"]["message.missing"].as_u64().unwrap_or(0) >= 1, "{r}");
        assert_eq!(r["conversation"], conv.as_str(), "같은 방에 반영된다");
        assert!(log_len(&a, &conv) > before);
        let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
    }

    #[test]
    fn ambiguous_room_can_become_a_new_room_or_be_skipped() {
        let (d, a) = (tmp("room"), tmp("room-arch"));
        let first = first_ingest(&d, &a);
        // 같은 제목, 내용이 거의 다른 내보내기 → 어느 방인지 모호
        let other = ["[가] [오전 7:00] 다른", "[나] [오전 7:01] 이야기", "[가] [오전 7:02] 완전히", "[나] [오전 7:03] 달라요"];
        let e2 = export(&d, "e2.txt", "방", "2026-10-02 12:00:00", &other);
        let mut ask = Script::new(&[]);
        assert!(matches!(ingest_interactive(&a, &e2, &mut ask), Flow::Skipped), "기본은 반영하지 않기");
        assert!(ask.asked[0].contains("정하지 못했습니다") && ask.asked[0].contains(&first));
        let mut ask = Script::new(&[1]); // 후보 1개 다음이 '새 방으로 따로 저장하기'
        let Flow::Done(r) = ingest_interactive(&a, &e2, &mut ask) else { panic!() };
        let new_id = r["conversation"].as_str().unwrap().to_string();
        assert_ne!(new_id, first);
        assert_eq!(fs::read_dir(&a).unwrap().count(), 2, "방이 둘이 된다");
        let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
    }

    #[test]
    fn ambiguous_room_can_join_an_existing_room() {
        let (d, a) = (tmp("join"), tmp("join-arch"));
        // 대량 소실 기준(활성 4개 이상) 미만의 작은 방
        let e1 = export(&d, "e1.txt", "방", "2026-10-02 10:00:00", &[SIX[0], SIX[1], SIX[2]]);
        let Flow::Done(r1) = ingest_interactive(&a, &e1, &mut Script::new(&[])) else { panic!() };
        let first = r1["conversation"].as_str().unwrap().to_string();
        let other = ["[가] [오전 7:00] 다른", "[나] [오전 7:01] 이야기", "[가] [오전 7:02] 완전히", "[나] [오전 7:03] 달라요"];
        let e2 = export(&d, "e2.txt", "방", "2026-10-02 12:00:00", &other);
        let Flow::Done(r) = ingest_interactive(&a, &e2, &mut Script::new(&[0])) else { panic!() };
        assert_eq!(r["conversation"], first.as_str());
        assert_eq!(fs::read_dir(&a).unwrap().count(), 1);
        let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
    }

    #[test]
    fn weak_rename_asks_and_applies_the_answer() {
        let chat = ["[철수] [오전 9:00] 하나", "[민수] [오전 9:01] 둘", "[민수] [오전 9:02] 셋"];
        let renamed = ["[철수샘] [오전 9:00] 하나", "[민수] [오전 9:01] 둘", "[민수] [오전 9:02] 셋"];
        for (answer, expect_renamed) in [(0usize, true), (1usize, false)] {
            let (d, a) = (tmp(&format!("rn{answer}")), tmp(&format!("rn{answer}-arch")));
            let e1 = export(&d, "e1.txt", "방", "2026-10-02 10:00:00", &chat);
            assert!(matches!(ingest_interactive(&a, &e1, &mut Script::new(&[])), Flow::Done(_)));
            let e2 = export(&d, "e2.txt", "방", "2026-10-02 10:10:00", &renamed);
            let mut ask = Script::new(&[answer]);
            let Flow::Done(r) = ingest_interactive(&a, &e2, &mut ask) else { panic!() };
            assert_eq!(ask.asked.len(), 1, "{:?}", ask.asked);
            assert!(ask.asked[0].contains("'철수' 님이 '철수샘' 님으로"));
            assert_eq!(r["events"]["participant.renamed"].as_u64().unwrap_or(0) == 1, expect_renamed, "{r}");
            let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
        }
    }

    #[test]
    fn suspect_extraction_reads_the_warning_text() {
        let w = json!(["발신자 이름 변경 의심 '철수' -> '철수샘' (근거 1건)을 확정하지 않았다: 근거가 부족하다. 맞다면 --accept-rename '철수=철수샘' 로 확인하라", "다른 경고"]);
        assert_eq!(extract_suspects(&w), [("철수".to_string(), "철수샘".to_string(), "근거가 부족하다".to_string())]);
        let none = json!(["발신자 이름 변경 의심 '철수' -> None (근거 0건)을 확정하지 않았다: x. 맞다면 --accept-rename '철수=None' 로 확인하라"]);
        assert!(extract_suspects(&none).is_empty(), "새 이름을 모르면 묻지 않는다");
        assert!(extract_suspects(&json!(null)).is_empty());
    }

    #[test]
    fn plain_message_drops_command_line_hints() {
        let m = "활성 메시지 10개 중 10개가 사라졌다. 이 PC의 대화 내역이 지워졌거나 내보내기 범위가 달라졌을 수 있다. 확인 후 --force로 다시 실행.";
        assert_eq!(plain_message(m), "활성 메시지 10개 중 10개가 사라졌다. 이 PC의 대화 내역이 지워졌거나 내보내기 범위가 달라졌을 수 있다.");
        let r = "제목 'x'과 일치하는 방을 하나로 정하지 못했다 (a: 겹침 1/5). 같은 이름의 다른 방이다. --conversation 으로 지정하라";
        assert_eq!(plain_message(r), "제목 'x'과 일치하는 방을 하나로 정하지 못했다 (a: 겹침 1/5). 같은 이름의 다른 방이다.");
        assert_eq!(plain_message("옵션 없는 문장."), "옵션 없는 문장.");
        assert_eq!(plain_message(""), "");
    }

    #[test]
    fn real_errors_are_reported_not_asked() {
        let (d, a) = (tmp("err"), tmp("err-arch"));
        let empty = d.join("empty.txt");
        fs::write(&empty, "").unwrap();
        let mut ask = Script::new(&[]);
        assert!(matches!(ingest_interactive(&a, &empty, &mut ask), Flow::Failed(_)));
        assert!(ask.asked.is_empty());
        let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
    }
}
