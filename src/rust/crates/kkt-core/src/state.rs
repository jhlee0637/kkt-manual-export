//! 이벤트 로그(events.jsonl)를 접어서(fold) 현재 상태를 만든다 (src/python/kkt/state.py 의 이식).
//!
//! 상태 파일은 따로 두지 않는다. 이벤트 로그가 유일한 원본이다. 트랜잭션은 종결 이벤트
//! (export.ingested / attach.committed / state.committed)로 닫고, 종결 이벤트 없이 끝난 꼬리(크래시)는 읽을 때 버린다.

use crate::error::{KktError, Result};
use crate::pyfmt::{dumps, strip};
use crate::SCHEMA_VERSION;
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::Path;

const TERMINATORS: [&str; 3] = ["export.ingested", "attach.committed", "state.committed"];

pub type Key = Vec<String>;

#[derive(Debug, Clone)]
pub struct Record {
    pub id: String,
    pub kind: String,
    pub date: String,
    pub hhmm: Option<String>,
    pub sender: Option<String>,
    pub text: String,
    pub content_type: String,
    pub status: String,
    pub version: u64,
    pub history: Vec<(String, u64)>,
    pub attachment_id: Option<String>,
    /// 연결된 사진 파일들 (한 메시지가 '사진 3장' 이면 최대 3개). `attachment_id` 는 그중 첫 번째
    pub attachment_ids: Vec<String>,
    /// 사진 메시지의 사진 수 (기본 1)
    pub image_count: u64,
    pub first_observed_at: Option<String>,
    pub participant_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Participant {
    pub id: String,
    pub names: Vec<String>,
    pub current: String,
}

#[derive(Debug, Clone)]
pub struct IngestInfo {
    pub export_name: String,
    pub export_sha256: String,
    pub saved_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub attachment_id: String,
    pub taken_at: String,
    pub sha256: String,
    pub filename: String,
    pub aliases: Vec<String>,
    pub mime_type: String,
}

#[derive(Debug, Default)]
pub struct State {
    pub registry: IndexMap<String, Record>,
    pub visible: Vec<String>,
    pub ingests: Vec<IngestInfo>,
    pub attachments: IndexMap<String, Attachment>,
    pub participants: IndexMap<String, Participant>,
    pub titles: Vec<String>,
    pub title: Option<String>,
    pub seq: u64,
}

/// 시스템 줄 안의 이름을 치환한다. 시스템 줄에서 이름은 '<이름>님' 꼴로 나온다. '.' 같은 짧은 이름이
/// 문장부호와 섞이지 않도록 '님'까지 묶어서 바꾼다.
pub fn relabel_text(text: &str, old: &str, new: &str) -> String {
    text.replace(&format!("{old}님"), &format!("{new}님"))
}

/// 발신자의 정체. 이름은 바뀔 수 있으므로 participant_id 를 쓴다 (없으면 이름으로 대신한다).
pub fn ident_of(rec: &Record) -> String {
    match rec.participant_id.as_deref() {
        Some(p) if !p.is_empty() => format!("p:{p}"),
        _ => format!("n:{}", rec.sender.as_deref().unwrap_or("None")),
    }
}

/// 내보내기 항목과 기존 기록을 맞추는 키. `reconcile::entry_key` 와 같은 모양이어야 한다.
pub fn key_of(rec: &Record) -> Key {
    if rec.kind == "message" {
        if rec.status == "deleted_for_everyone" {
            return vec!["d".into(), rec.date.clone()];
        }
        return vec!["m".into(), rec.date.clone(), rec.hhmm.clone().unwrap_or_default(), ident_of(rec), rec.text.clone()];
    }
    if rec.kind == "deleted_marker" {
        return vec!["d".into(), rec.date.clone()];
    }
    vec!["s".into(), rec.date.clone(), rec.text.clone()]
}

fn sv(ev: &Value, k: &str) -> Result<String> {
    ev.get(k)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| KktError::new("bad_log", format!("이벤트에 문자열 필드 {k:?} 가 없다")))
}

fn so(ev: &Value, k: &str) -> Option<String> {
    ev.get(k).and_then(|v| v.as_str()).map(|s| s.to_string())
}

impl State {
    pub fn current_names(&self) -> IndexMap<String, String> {
        self.participants.iter().map(|(k, p)| (k.clone(), p.current.clone())).collect()
    }

    /// {이름: participant_id}. 옛 이름도 포함한다 (이름이 새 메시지에만 바뀌는 경우에도 같은 사람으로 알아본다).
    /// 두 참가자가 같은 이름을 가졌던 적이 있으면 그 이름은 애매하므로 뺀다.
    pub fn name_index(&self) -> HashMap<String, String> {
        let mut idx: HashMap<String, String> = HashMap::new();
        let mut dup: HashSet<String> = HashSet::new();
        for (pid, p) in &self.participants {
            for n in &p.names {
                if let Some(existing) = idx.get(n) {
                    if existing != pid {
                        dup.insert(n.clone());
                    }
                }
                idx.insert(n.clone(), pid.clone());
            }
        }
        for n in dup {
            idx.remove(&n);
        }
        idx
    }

    pub fn known_export(&self, sha256: &str) -> bool {
        self.ingests.iter().any(|i| i.export_sha256 == sha256)
    }

    fn rec_mut(&mut self, ev: &Value) -> Result<&mut Record> {
        let id = sv(ev, "message_id")?;
        self.registry.get_mut(&id).ok_or_else(|| KktError::new("bad_log", format!("알 수 없는 메시지 ID: {id}")))
    }

    pub fn apply(&mut self, ev: &Value) -> Result<()> {
        let t = sv(ev, "type")?;
        match t.as_str() {
            "message.observed" | "deleted_marker.observed" | "system.observed" => {
                let id = sv(ev, "message_id")?;
                self.registry.insert(
                    id.clone(),
                    Record {
                        id,
                        kind: sv(ev, "kind")?,
                        date: sv(ev, "date")?,
                        hhmm: so(ev, "hhmm"),
                        sender: so(ev, "sender"),
                        text: sv(ev, "text")?,
                        content_type: so(ev, "content_type").unwrap_or_else(|| "text".to_string()),
                        status: "active".to_string(),
                        version: 1,
                        history: Vec::new(),
                        attachment_id: None,
                        attachment_ids: Vec::new(),
                        image_count: ev.get("image_count").and_then(|v| v.as_u64()).unwrap_or(1),
                        first_observed_at: so(ev, "observed_at"),
                        participant_id: so(ev, "participant_id"),
                    },
                );
            }
            "message.deleted_for_everyone" => self.rec_mut(ev)?.status = "deleted_for_everyone".to_string(),
            "message.missing" => {
                let covered = ev.get("range_covered").and_then(|v| v.as_bool()).unwrap_or(false);
                self.rec_mut(ev)?.status = if covered { "missing" } else { "unverifiable" }.to_string();
            }
            "message.reappeared" => self.rec_mut(ev)?.status = "active".to_string(),
            "message.edit_candidate" => {
                let cur = sv(ev, "current_text")?;
                let r = self.rec_mut(ev)?;
                r.history.push((r.text.clone(), r.version));
                r.text = cur;
                r.version += 1;
            }
            "conversation.title_observed" | "conversation.renamed" => {
                let new = if t == "conversation.title_observed" { sv(ev, "title")? } else { sv(ev, "to")? };
                self.titles.push(new.clone());
                self.title = Some(new);
            }
            "participant.observed" => {
                let pid = sv(ev, "participant_id")?;
                let name = sv(ev, "name")?;
                self.participants.insert(pid.clone(), Participant { id: pid, names: vec![name.clone()], current: name });
            }
            "participant.renamed" => {
                let pid = sv(ev, "participant_id")?;
                let (from, to) = (sv(ev, "from")?, sv(ev, "to")?);
                let p = self.participants.get_mut(&pid).ok_or_else(|| KktError::new("bad_log", "알 수 없는 참가자"))?;
                if !p.names.contains(&to) {
                    p.names.push(to.clone());
                }
                p.current = to.clone();
                // 이름 변경은 내보내기에서 소급되어 시스템 줄 본문에도 반영된다 (실측). 기록도 같이 맞춘다.
                for r in self.registry.values_mut() {
                    if r.kind == "system" {
                        let new_text = relabel_text(&r.text, &from, &to);
                        if new_text != r.text {
                            r.history.push((r.text.clone(), r.version));
                            r.text = new_text;
                            r.version += 1;
                        }
                    }
                }
            }
            "participant.linked" => {
                let keep_id = sv(ev, "participant_id")?;
                let gone_id = sv(ev, "merged_participant_id")?;
                let gone = self
                    .participants
                    .shift_remove(&gone_id)
                    .ok_or_else(|| KktError::new("bad_log", "알 수 없는 참가자"))?;
                let keep = self.participants.get_mut(&keep_id).ok_or_else(|| KktError::new("bad_log", "알 수 없는 참가자"))?;
                for n in gone.names {
                    if !keep.names.contains(&n) {
                        keep.names.push(n);
                    }
                }
                keep.current = sv(ev, "current_name")?;
                for r in self.registry.values_mut() {
                    if r.participant_id.as_deref() == Some(gone_id.as_str()) {
                        r.participant_id = Some(keep_id.clone());
                    }
                }
            }
            "attachment.saved" => {
                let id = sv(ev, "attachment_id")?;
                let aliases: Vec<String> = ev
                    .get("aliases")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                self.attachments.insert(
                    id.clone(),
                    Attachment {
                        attachment_id: id,
                        taken_at: sv(ev, "taken_at")?,
                        sha256: sv(ev, "sha256")?,
                        filename: sv(ev, "filename")?,
                        aliases,
                        mime_type: so(ev, "mime_type").unwrap_or_default(),
                    },
                );
            }
            "attachment.linked" => {
                let aid = sv(ev, "attachment_id")?;
                let r = self.rec_mut(ev)?;
                r.attachment_ids.push(aid);
                r.attachment_id = Some(r.attachment_ids[0].clone());
            }
            "export.ingested" => {
                self.visible = ev
                    .get("visible")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                self.ingests.push(IngestInfo { export_name: sv(ev, "export_name")?, export_sha256: sv(ev, "export_sha256")?, saved_at: so(ev, "saved_at") });
            }
            "attach.committed" | "state.committed" => {}
            other => return Err(KktError::new("bad_log", format!("알 수 없는 이벤트 타입: {other}"))),
        }
        let eid = sv(ev, "event_id")?;
        let n: u64 = eid.split('_').nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| KktError::new("bad_log", "event_id 형식 오류"))?;
        self.seq = self.seq.max(n);
        Ok(())
    }
}

/// 로그는 항상 \n 으로 끝나는 줄들이다 (Python `_log_lines`).
fn log_lines(text: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = text.split('\n').collect();
    if parts.last().map(|s| s.is_empty()).unwrap_or(false) {
        parts.pop();
    }
    parts
}

/// (상태, 유효한 줄 수). 종결 이벤트 뒤의 꼬리 줄은 무시한다.
pub fn load(events_path: &Path) -> Result<(State, usize)> {
    let mut st = State::default();
    if !events_path.exists() {
        return Ok((st, 0));
    }
    let text = fs::read_to_string(events_path)?;
    let mut pending: Vec<Value> = Vec::new();
    let mut valid = 0usize;
    for (i, line) in log_lines(&text).into_iter().enumerate() {
        let n = i + 1;
        if strip(line).is_empty() {
            continue;
        }
        let ev: Value = serde_json::from_str(line)?;
        let ver = ev.get("schema_version").and_then(|v| v.as_u64()).unwrap_or(1);
        if ver < SCHEMA_VERSION {
            return Err(KktError::new(
                "schema_too_old",
                format!(
                    "{} 는 스키마 v{ver} 로그다 (현재 v{SCHEMA_VERSION}). 로그는 raw 내보내기에서 다시 만들 수 있다: 이 폴더를 지우고 ingest 를 다시 실행하라",
                    events_path.display()
                ),
            ));
        }
        let is_term = ev.get("type").and_then(|v| v.as_str()).map(|t| TERMINATORS.contains(&t)).unwrap_or(false);
        pending.push(ev);
        if is_term {
            for e in pending.drain(..) {
                st.apply(&e)?;
            }
            valid = n;
        }
    }
    Ok((st, valid))
}

/// 트랜잭션을 추가한다. 이전의 불완전한 꼬리가 있으면 먼저 잘라낸다.
pub fn append(events_path: &Path, valid_lines: usize, events: &[Value]) -> Result<()> {
    if let Some(parent) = events_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if events_path.exists() {
        let text = fs::read_to_string(events_path)?;
        let lines = log_lines(&text);
        if lines.len() > valid_lines {
            let kept: String = lines[..valid_lines].iter().map(|l| format!("{l}\n")).collect();
            fs::write(events_path, kept)?;
        }
    }
    let mut f = fs::OpenOptions::new().create(true).append(true).open(events_path)?;
    for ev in events {
        f.write_all(dumps(ev).as_bytes())?;
        f.write_all(b"\n")?;
    }
    f.flush()?;
    f.sync_all()?;
    Ok(())
}
