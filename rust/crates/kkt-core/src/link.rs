//! 새 내보내기가 어느 대화방에 속하는지 판정한다 (kkt/link.py 의 이식).
//!
//! 방 제목은 바뀔 수 있고 TXT 에는 방 ID 가 없다. 그래서 제목 일치 여부와 함께 '이전에 본 메시지들이 새 내보내기에도
//! 같은 순서로 있는가'(겹침 비율)를 증거로 쓴다. 발신자 이름도 바뀔 수 있으므로 겹침은 (날짜, 분, 내용)으로만 비교한다.
//! 애매하면 중단한다. 잘못 합치면 두 방의 기록이 섞여 되돌리기 어렵기 때문이다.
//!
//! 임계값 비교는 Python 과 같은 부동소수점 연산(`m/c >= 0.6`)을 쓴다. 정수 비교로 바꾸면 경계에서 달라질 수 있다.

use crate::difflib::SequenceMatcher;
use crate::error::{KktError, Result};
use crate::parse::{Kind, ParsedExport};
use crate::pyfmt::repr_opt;
use crate::reconcile::sha1_hex;
use crate::state::{self, State};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub const MIN_COMPARE: usize = 3;
pub const SAME_RATIO: f64 = 0.5;
pub const RENAME_RATIO: f64 = 0.6;
pub const OTHER_MAX_RATIO: f64 = 0.3;

/// (맞은 수, 비교 대상 수). 새 내보내기의 첫 날짜 이후의 이전 활성 메시지만 비교한다.
pub fn overlap(state: &State, parsed: &ParsedExport) -> (usize, usize) {
    let first = match parsed.first_date.as_deref() {
        Some(f) if !f.is_empty() => f,
        _ => return (0, 0),
    };
    let old: Vec<(String, String, String)> = state
        .visible
        .iter()
        .filter_map(|i| state.registry.get(i))
        .filter(|r| r.kind == "message" && r.status == "active" && r.date.as_str() >= first)
        .map(|r| (r.date.clone(), r.hhmm.clone().unwrap_or_default(), r.text.clone()))
        .collect();
    let new: Vec<(String, String, String)> = parsed
        .entries
        .iter()
        .filter(|e| e.kind == Kind::Message)
        .map(|e| (e.date.clone(), e.hhmm.clone().unwrap_or_default(), e.text.clone()))
        .collect();
    let m = SequenceMatcher::new(&old, &new).get_matching_blocks().iter().map(|b| b.2).sum();
    (m, old.len())
}

pub fn new_conversation_id(parsed: &ParsedExport) -> String {
    // Python 의 f"{parsed.title}|{parsed.saved_at}" 는 None 을 'None' 으로 찍는다.
    let t = parsed.title.as_deref().unwrap_or("None");
    let s = parsed.saved_at.as_deref().unwrap_or("None");
    format!("kt_{}", &sha1_hex(&format!("{t}|{s}"))[..10])
}

fn fmt(cid: &str, m: usize, c: usize) -> String {
    if c == 0 {
        format!("{cid}: 겹침 {m}/{c}")
    } else {
        format!("{cid}: 겹침 {m}/{c} ({:.0}%)", 100.0 * m as f64 / c as f64)
    }
}

struct Cand {
    id: String,
    m: usize,
    c: usize,
    r: Option<f64>,
    title_match: bool,
}

fn link(status: &str, basis: &str, cand: Option<&Cand>) -> Value {
    json!({"status": status, "basis": basis,
           "matched": cand.map_or(0, |c| c.m), "compared": cand.map_or(0, |c| c.c)})
}

/// (conversation_id, link). link 는 conversation.renamed 이벤트에 근거로 남는다.
pub fn resolve(root: &Path, parsed: &ParsedExport, hint: Option<&str>, force: bool) -> Result<(String, Value)> {
    let mut cands: Vec<Cand> = Vec::new();
    if root.exists() {
        let mut dirs: Vec<_> = fs::read_dir(root)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("events.jsonl").exists()).collect();
        dirs.sort();
        for d in dirs {
            let (state, _) = state::load(&d.join("events.jsonl"))?;
            let (m, c) = overlap(&state, parsed);
            cands.push(Cand {
                id: d.file_name().unwrap().to_string_lossy().to_string(),
                m,
                c,
                r: if c == 0 { None } else { Some(m as f64 / c as f64) },
                title_match: parsed.title.as_ref().map_or(false, |t| state.titles.contains(t)),
            });
        }
    }

    if let Some(hint) = hint.filter(|h| !h.is_empty()) {
        let cand = cands.iter().find(|c| c.id == hint);
        if let Some(c) = cand {
            if c.c >= MIN_COMPARE && c.r.map_or(false, |r| r < OTHER_MAX_RATIO) && !force {
                return Err(KktError::new(
                    "explicit_conversation_mismatch",
                    format!(
                        "--conversation {hint} 와 이 내보내기가 거의 겹치지 않는다 ({}). 다른 방의 내보내기일 수 있다. 확실하면 --force",
                        fmt(hint, c.m, c.c)
                    ),
                ));
            }
        }
        return Ok((hint.to_string(), link("explicit", "explicit_conversation", cand)));
    }

    let tm: Vec<&Cand> = cands.iter().filter(|c| c.title_match).collect();
    if !tm.is_empty() {
        let ok: Vec<&&Cand> = tm.iter().filter(|c| c.r.is_none() || c.c < MIN_COMPARE || c.r.map_or(false, |r| r >= SAME_RATIO)).collect();
        if ok.len() == 1 {
            return Ok((ok[0].id.clone(), link("same", "title_match", Some(ok[0]))));
        }
        return Err(KktError::new(
            "room_title_ambiguous",
            format!(
                "제목 {}과 일치하는 방을 하나로 정하지 못했다 ({}). 같은 이름의 다른 방이거나 내용이 크게 바뀌었을 수 있다. --conversation 으로 지정하라",
                repr_opt(parsed.title.as_deref()),
                tm.iter().map(|c| fmt(&c.id, c.m, c.c)).collect::<Vec<_>>().join("; ")
            ),
        ));
    }

    // 파이썬의 sorted(..., reverse=True) 는 안정 정렬이다 (같은 값은 원래 순서를 유지한다).
    let mut scored: Vec<&Cand> = cands.iter().filter(|c| c.c >= MIN_COMPARE).collect();
    scored.sort_by(|a, b| b.r.partial_cmp(&a.r).unwrap_or(std::cmp::Ordering::Equal));
    if let Some(best) = scored.first() {
        let second = scored.get(1);
        let br = best.r.unwrap_or(0.0);
        if br >= RENAME_RATIO && best.m >= MIN_COMPARE && second.map_or(true, |s| s.r.map_or(true, |r| r < OTHER_MAX_RATIO)) {
            return Ok((best.id.clone(), link("renamed_room", "message_overlap", Some(best))));
        }
        if br >= OTHER_MAX_RATIO {
            return Err(KktError::new(
                "room_rename_ambiguous",
                format!(
                    "제목 {}은 처음 보는 이름인데 기존 방과 일부 겹친다 ({}). 이름이 바뀐 같은 방인지 확정할 수 없다. --conversation 으로 지정하라",
                    repr_opt(parsed.title.as_deref()),
                    scored.iter().take(3).map(|c| fmt(&c.id, c.m, c.c)).collect::<Vec<_>>().join("; ")
                ),
            ));
        }
    }
    Ok((new_conversation_id(parsed), json!({"status": "new", "basis": "no_overlap", "matched": 0, "compared": 0})))
}
