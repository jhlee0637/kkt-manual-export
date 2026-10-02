//! 새 내보내기를 이전 상태와 비교해 이벤트를 만든다 (kkt/reconcile.py 의 이식).
//!
//! 카카오톡 내보내기에는 안정적인 메시지 ID가 없다. 그래서 직전 내보내기에 보인 항목들과 순서 기준
//! (SequenceMatcher)으로 맞춘다. 관측된 사실만 확정으로 기록하고, 나머지는 후보/미확인으로 둔다.
//!
//! 판정 (test 방에서 실측한 동작에 근거):
//! - '모두에게 삭제'  -> 같은 자리에 '메시지가 삭제되었습니다.' 줄이 남는다.  => deleted_for_everyone (확정)
//! - '나에게서만 삭제' -> 줄이 아예 사라진다. 삭제 주체를 알 수 없다.        => missing (사유 불명)
//! - 수정             -> 표시 없이 같은 시각/보낸이로 내용만 바뀐다.         => edit_candidate (후보)
//!
//! 이벤트가 나오는 순서와 삽입 순서에 의존하는 순회는 Python 구현과 같아야 한다 (바이트 단위로 비교된다).

use crate::difflib::{SequenceMatcher, Tag};
use crate::error::{KktError, Result};
use crate::parse::{Entry, Kind, ParsedExport};
use crate::pyfmt::{repr, repr_opt};
use crate::state::{ident_of, key_of, relabel_text, Key, Record, State};
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};

/// 자동 확정에 필요한 '같은 시각/내용인데 이름만 다른' 근거 수.
pub const MIN_RENAME_EVIDENCE: usize = 2;

pub fn sha1_hex(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn iso(date: &str, hhmm: &str) -> String {
    format!("{date}T{hhmm}:00+09:00")
}

fn content(e: &Entry) -> Value {
    match e.content_type.as_str() {
        "image" => json!([{"type": "image", "attachment_id": null}]),
        "emoticon" => json!([{"type": "emoticon"}]),
        _ => json!([{"type": "text", "text": e.text}]),
    }
}

/// 삭제 표식처럼 시각이 없는 줄의 시각 범위를 앞뒤 메시지로 추정한다 -> (inferred_after, inferred_before).
fn neighbors(entries: &[Entry], j: usize) -> (Value, Value) {
    let prev = entries[..j]
        .iter()
        .rev()
        .find_map(|e| e.hhmm.as_deref().filter(|h| !h.is_empty()).map(|h| iso(&e.date, h)));
    let next = entries[j + 1..]
        .iter()
        .find_map(|e| e.hhmm.as_deref().filter(|h| !h.is_empty()).map(|h| iso(&e.date, h)));
    (prev.map(Value::from).unwrap_or(Value::Null), next.map(Value::from).unwrap_or(Value::Null))
}

fn ident_entry(e: &Entry, idx: &HashMap<String, String>) -> String {
    match e.sender.as_deref().and_then(|s| idx.get(s)).filter(|p| !p.is_empty()) {
        Some(p) => format!("p:{p}"),
        None => format!("n:{}", e.sender.as_deref().unwrap_or("None")),
    }
}

/// `state::key_of` 와 같은 모양. 발신자는 이름이 아니라 참가자로 환산해서 비교한다.
pub fn entry_key(e: &Entry, idx: &HashMap<String, String>) -> Key {
    match e.kind {
        Kind::Message => vec![
            "m".into(),
            e.date.clone(),
            e.hhmm.clone().unwrap_or_default(),
            ident_entry(e, idx),
            e.text.clone(),
        ],
        Kind::DeletedMarker => vec!["d".into(), e.date.clone()],
        Kind::System => vec!["s".into(), e.date.clone(), e.text.clone()],
    }
}

#[derive(Debug)]
pub struct Confirmed {
    pub participant_id: String,
    pub old_name: String,
    pub new_name: String,
    pub matched: usize,
    pub messages: usize,
    pub system_lines: usize,
    pub basis: &'static str,
}

#[derive(Debug)]
pub struct Suspect {
    pub old_name: String,
    pub new_name: Option<String>,
    pub matched: usize,
    pub problems: Vec<String>,
}

/// 멤버 이름 변경을 찾는다. (확정 목록, 확정하지 못한 의심 목록).
///
/// 확정 조건 (모두 만족):
/// - 그 참가자의 정렬된 메시지가 전부 같은 새 이름 하나로만 바뀌었다
/// - 그 참가자의 어떤 이름도 새 내보내기의 발신자로 더 이상 나오지 않는다 (변경이 완결됨)
/// - 새 이름이 이 방의 다른 참가자가 쓰던 이름이 아니다 (두 사람이 합쳐지는 것을 막는다)
/// - 서로 다른 참가자 둘이 같은 새 이름으로 가지 않는다
/// - 근거가 MIN_RENAME_EVIDENCE 건 이상 (또는 accept 로 확인됨).
///   근거 = 이름만 달라진 정렬된 메시지 수 + 같은 치환이 일어난 시스템 줄 수
pub fn detect_renames(state: &State, parsed: &ParsedExport, accept: &IndexMap<String, String>) -> (Vec<Confirmed>, Vec<Suspect>) {
    let idx = state.name_index();
    let cur = state.current_names();
    let old: Vec<&Record> = state
        .visible
        .iter()
        .filter_map(|i| state.registry.get(i))
        .filter(|r| r.kind == "message" && r.status == "active" && r.participant_id.as_deref().map_or(false, |p| !p.is_empty()))
        .collect();
    let new: Vec<&Entry> = parsed.entries.iter().filter(|e| e.kind == Kind::Message).collect();
    let a: Vec<(String, String, String)> =
        old.iter().map(|r| (r.date.clone(), r.hhmm.clone().unwrap_or_default(), r.text.clone())).collect();
    let b: Vec<(String, String, String)> =
        new.iter().map(|e| (e.date.clone(), e.hhmm.clone().unwrap_or_default(), e.text.clone())).collect();
    let sm = SequenceMatcher::new(&a, &b);

    // pid -> {새 내보내기에 찍힌 이름: 정렬된 메시지 수}
    let mut per_pid: IndexMap<String, IndexMap<String, usize>> = IndexMap::new();
    for (i, j, n) in sm.get_matching_blocks() {
        for k in 0..n {
            let pid = old[i + k].participant_id.clone().unwrap_or_default();
            let sender = new[j + k].sender.clone().unwrap_or_default();
            *per_pid.entry(pid).or_default().entry(sender).or_insert(0) += 1;
        }
    }
    // 두 번째 근거: 시스템 줄 본문의 이름도 같은 치환이 일어났는가
    let old_sys: Vec<&Record> = state.visible.iter().filter_map(|i| state.registry.get(i)).filter(|r| r.kind == "system").collect();
    let mut new_sys: HashMap<&str, usize> = HashMap::new();
    for e in parsed.entries.iter().filter(|e| e.kind == Kind::System) {
        *new_sys.entry(e.text.as_str()).or_insert(0) += 1;
    }
    let new_senders: HashSet<&str> = new.iter().filter_map(|e| e.sender.as_deref()).collect();

    // 이 참가자의 정렬된 메시지가 현재 이름이 아닌 이름 하나로만 찍혔다면 이름이 바뀐 것이다.
    let mut target_of: IndexMap<&String, &String> = IndexMap::new();
    for (pid, ctr) in &per_pid {
        if ctr.len() == 1 {
            let name = ctr.keys().next().unwrap();
            if cur.get(pid) != Some(name) {
                target_of.insert(pid, name);
            }
        }
    }
    let mut by_target: HashMap<&String, usize> = HashMap::new();
    for v in target_of.values() {
        *by_target.entry(*v).or_insert(0) += 1;
    }

    let mut confirmed = Vec::new();
    let mut suspects = Vec::new();
    for (pid, ctr) in &per_pid {
        let o = cur.get(pid).cloned().unwrap_or_default();
        // 이 사람이 모르는 이름이 나타나지 않았고, 하나의 옛 이름으로 통째로 바뀐 것도 아니면 정상이다.
        let unknown = ctr.keys().any(|name| idx.get(name) != Some(pid));
        let single_other = ctr.len() == 1 && ctr.keys().next().map_or(false, |n| *n != o);
        if !unknown && !single_other {
            continue;
        }
        let n: Option<&String> = target_of.get(pid).copied();
        let owns = n.map_or(false, |n| idx.get(n) == Some(pid));
        let c = n.map_or(0, |n| ctr[n]);
        let mut sys_ev = 0usize;
        if let Some(n) = n {
            for r in &old_sys {
                let t = relabel_text(&r.text, &o, n);
                if t != r.text && new_sys.get(t.as_str()).copied().unwrap_or(0) > 0 {
                    sys_ev += 1;
                }
            }
        }
        let (msg_ev, c) = (c, c + sys_ev);
        let mut problems: Vec<String> = Vec::new();
        if n.is_none() {
            problems.push("이 사람의 메시지가 여러 이름으로 갈라졌다".to_string());
        }
        let own_names = state.participants.get(pid).map(|p| p.names.clone()).unwrap_or_default();
        if own_names.iter().any(|name| Some(name) != n && new_senders.contains(name.as_str())) {
            problems.push("옛 이름이 새 내보내기에도 발신자로 남아 있다".to_string());
        }
        if let Some(n) = n {
            if let Some(owner) = idx.get(n) {
                if owner != pid {
                    problems.push("새 이름이 이미 이 방의 다른 참가자 이름이다".to_string());
                }
            }
            if by_target.get(n).copied().unwrap_or(0) > 1 {
                problems.push("서로 다른 참가자 둘이 같은 새 이름으로 간다".to_string());
            }
        }
        let weak = n.is_some() && c < MIN_RENAME_EVIDENCE;
        let forced = n.map_or(false, |n| accept.get(&o) == Some(n));
        if problems.is_empty() && (!weak || forced) {
            confirmed.push(Confirmed {
                participant_id: pid.clone(),
                old_name: o,
                new_name: n.unwrap().clone(),
                matched: c,
                messages: msg_ev,
                system_lines: sys_ev,
                basis: if forced {
                    "forced_by_user"
                } else if owns {
                    "restored_previous_name"
                } else {
                    "consistent_relabel"
                },
            });
        } else if !owns {
            suspects.push(Suspect { old_name: o, new_name: n.cloned(), matched: c, problems });
        }
    }
    (confirmed, suspects)
}

struct Ctx<'a> {
    state: &'a State,
    conv: &'a str,
    observed_at: Value,
    sha: &'a str,
    events: Vec<Value>,
    used: HashSet<String>,
    idx: HashMap<String, String>,
}

impl<'a> Ctx<'a> {
    fn emit(&mut self, t: &str, fields: Vec<(&str, Value)>) {
        let mut m = Map::new();
        m.insert("type".into(), t.into());
        m.insert("conversation_id".into(), self.conv.into());
        m.insert("observed_at".into(), self.observed_at.clone());
        m.insert("export_sha256".into(), self.sha.into());
        for (k, v) in fields {
            m.insert(k.to_string(), v);
        }
        self.events.push(Value::Object(m));
    }

    fn new_id(&mut self, prefix: &str, parts: &[&str]) -> String {
        let mut n = 0usize;
        loop {
            let mut pieces: Vec<String> = vec![self.conv.to_string()];
            pieces.extend(parts.iter().map(|s| s.to_string()));
            pieces.push(n.to_string());
            let h = sha1_hex(&pieces.join("|"));
            let cid = format!("{prefix}_{}", &h[..16]);
            if !self.state.registry.contains_key(&cid) && !self.used.contains(&cid) {
                self.used.insert(cid.clone());
                return cid;
            }
            n += 1;
        }
    }

    fn pid_for(&mut self, name: &str) -> String {
        if let Some(p) = self.idx.get(name) {
            return p.clone();
        }
        let pid = self.new_id("kp", &["participant", name]);
        self.idx.insert(name.to_string(), pid.clone());
        self.emit("participant.observed", vec![("participant_id", pid.clone().into()), ("name", name.into())]);
        pid
    }

    /// equal 이 아닌 구간 하나를 처리한다.
    fn block(&mut self, old_block: &[String], new_block: &[usize], new: &[Entry], assigned: &mut [Option<String>], vanished: &mut Vec<String>) {
        let state = self.state;
        let reg = &state.registry;
        let mut old_left: Vec<String> = old_block.to_vec();
        let mut new_left: Vec<usize> = new_block.to_vec();

        // 1) 수정 후보: 같은 날짜/분/보낸이이고 내용만 다른 쌍. 양쪽 개수가 같을 때만 순서대로 짝짓는다.
        let mut g_old: IndexMap<(String, String, String), Vec<String>> = IndexMap::new();
        for oid in &old_left {
            let r = &reg[oid];
            if r.kind == "message" && r.status == "active" {
                g_old.entry((r.date.clone(), r.hhmm.clone().unwrap_or_default(), ident_of(r))).or_default().push(oid.clone());
            }
        }
        let mut g_new: IndexMap<(String, String, String), Vec<usize>> = IndexMap::new();
        for &j in &new_left {
            let e = &new[j];
            if e.kind == Kind::Message {
                g_new.entry((e.date.clone(), e.hhmm.clone().unwrap_or_default(), ident_entry(e, &self.idx))).or_default().push(j);
            }
        }
        for (trip, olist) in &g_old {
            let empty = Vec::new();
            let nlist = g_new.get(trip).unwrap_or(&empty);
            if olist.len() != nlist.len() {
                continue;
            }
            for (oid, &j) in olist.iter().zip(nlist.iter()) {
                assigned[j] = Some(oid.clone());
                if let Some(p) = old_left.iter().position(|x| x == oid) {
                    old_left.remove(p);
                }
                if let Some(p) = new_left.iter().position(|&x| x == j) {
                    new_left.remove(p);
                }
                if reg[oid].text != new[j].text {
                    self.emit(
                        "message.edit_candidate",
                        vec![
                            ("message_id", oid.clone().into()),
                            ("previous_text", reg[oid].text.clone().into()),
                            ("current_text", new[j].text.clone().into()),
                            ("basis", "same_position_minute_sender".into()),
                            ("confidence", "candidate".into()),
                        ],
                    );
                }
            }
        }

        // 2) 모두에게 삭제: 새 삭제 표식과 사라진 활성 메시지를 순서대로 짝짓는다.
        let dels: Vec<usize> = new_left.iter().copied().filter(|&j| new[j].kind == Kind::DeletedMarker).collect();
        let olds: Vec<String> = old_left
            .iter()
            .filter(|oid| reg[*oid].kind == "message" && reg[*oid].status == "active")
            .cloned()
            .collect();
        for (&j, oid) in dels.iter().zip(olds.iter()) {
            assigned[j] = Some(oid.clone());
            if let Some(p) = new_left.iter().position(|&x| x == j) {
                new_left.remove(p);
            }
            if let Some(p) = old_left.iter().position(|x| x == oid) {
                old_left.remove(p);
            }
            let (after, before) = neighbors(new, j);
            self.emit(
                "message.deleted_for_everyone",
                vec![
                    ("message_id", oid.clone().into()),
                    ("basis", "deleted_marker_in_place".into()),
                    ("inferred_after", after),
                    ("inferred_before", before),
                ],
            );
        }

        // 3) 남은 이전 항목은 사라진 것
        vanished.extend(old_left);

        // 4) 남은 새 항목은 새로 관측된 것 (이전에 사라졌던 같은 내용이면 다시 나타난 것)
        for j in new_left {
            let e = &new[j];
            let want = entry_key(e, &self.idx);
            let back = reg
                .iter()
                .find(|(i, r)| {
                    (r.status == "missing" || r.status == "unverifiable")
                        && key_of(r) == want
                        && !assigned.iter().any(|a| a.as_deref() == Some(i.as_str()))
                })
                .map(|(i, _)| i.clone());
            if let Some(back) = back {
                assigned[j] = Some(back.clone());
                self.emit("message.reappeared", vec![("message_id", back.into())]);
                continue;
            }
            let (after, before) = neighbors(new, j);
            let mid = match e.kind {
                Kind::Message => {
                    let hhmm = e.hhmm.clone().unwrap_or_default();
                    let sender = e.sender.clone().unwrap_or_default();
                    let mid = self.new_id("kmsg", &[&e.date, &hhmm, &sender, &e.text]);
                    let pid = self.pid_for(&sender); // Python 과 같은 순서: new_id -> pid_for(participant.observed) -> message.observed
                    self.emit(
                        "message.observed",
                        vec![
                            ("message_id", mid.clone().into()),
                            ("kind", "message".into()),
                            ("date", e.date.clone().into()),
                            ("hhmm", hhmm.clone().into()),
                            ("sender", sender.into()),
                            ("participant_id", pid.into()),
                            ("text", e.text.clone().into()),
                            ("content_type", e.content_type.clone().into()),
                            ("timestamp", iso(&e.date, &hhmm).into()),
                            ("timestamp_precision", "minute".into()),
                            ("ordinal", j.into()),
                            ("content", content(e)),
                            // TXT에는 답장 정보가 없다. 답장이 아니라는 뜻이 아니라 '알 수 없다'는 뜻이다.
                            ("reply_to", json!({"status": "unknown_from_txt"})),
                        ],
                    );
                    mid
                }
                Kind::DeletedMarker => {
                    let mid = self.new_id("kdel", &[&e.date, &j.to_string()]);
                    self.emit(
                        "deleted_marker.observed",
                        vec![
                            ("message_id", mid.clone().into()),
                            ("kind", "deleted_marker".into()),
                            ("date", e.date.clone().into()),
                            ("text", e.text.clone().into()),
                            ("timestamp", Value::Null),
                            ("timestamp_precision", "inferred".into()),
                            ("ordinal", j.into()),
                            ("note", "삭제 표식만 관측됨. 삭제 전 내용은 알 수 없다.".into()),
                            ("inferred_after", after),
                            ("inferred_before", before),
                        ],
                    );
                    mid
                }
                Kind::System => {
                    let mid = self.new_id("ksys", &[&e.date, &e.text]);
                    self.emit(
                        "system.observed",
                        vec![
                            ("message_id", mid.clone().into()),
                            ("kind", "system".into()),
                            ("date", e.date.clone().into()),
                            ("text", e.text.clone().into()),
                            ("ordinal", j.into()),
                            ("timestamp", Value::Null),
                            ("timestamp_precision", "inferred".into()),
                            ("inferred_after", after),
                            ("inferred_before", before),
                        ],
                    );
                    mid
                }
            };
            assigned[j] = Some(mid);
        }
    }
}

/// 이벤트 목록을 반환한다 (event_id 는 호출자가 부여). 상태는 바꾸지 않는다.
pub fn reconcile(
    state: &State,
    parsed: &ParsedExport,
    conversation_id: &str,
    export_name: &str,
    export_sha256: &str,
    force: bool,
    accept: &IndexMap<String, String>,
    link: Option<&Value>,
) -> Result<Vec<Value>> {
    let new = &parsed.entries;
    if new.is_empty() && !state.visible.is_empty() {
        return Err(KktError::new("empty_export", "새 내보내기에 항목이 없다. 빈 파일이거나 형식이 바뀌었을 수 있다."));
    }
    let observed_at: Value = parsed.saved_at.clone().map(Value::from).unwrap_or(Value::Null);
    let mut ctx = Ctx {
        state,
        conv: conversation_id,
        observed_at: observed_at.clone(),
        sha: export_sha256,
        events: Vec::new(),
        used: HashSet::new(),
        idx: HashMap::new(),
    };

    // 방 제목 이력
    if let Some(title) = parsed.title.as_deref().filter(|t| !t.is_empty()) {
        match &state.title {
            None => ctx.emit("conversation.title_observed", vec![("title", title.into())]),
            Some(prev) if prev != title => {
                let basis = link.and_then(|l| l.get("basis")).and_then(|b| b.as_str()).unwrap_or("explicit_conversation").to_string();
                ctx.emit(
                    "conversation.renamed",
                    vec![
                        ("from", prev.clone().into()),
                        ("to", title.into()),
                        ("basis", basis.into()),
                        ("link", link.cloned().unwrap_or_else(|| json!({}))),
                    ],
                );
            }
            _ => {}
        }
    }

    // 멤버 이름 변경
    let (confirmed, suspects) = detect_renames(state, parsed, accept);
    ctx.idx = state.name_index();
    for r in &confirmed {
        ctx.emit(
            "participant.renamed",
            vec![
                ("participant_id", r.participant_id.clone().into()),
                ("to", r.new_name.clone().into()),
                ("matched_messages", r.matched.into()),
                ("evidence", json!({"messages": r.messages, "system_lines": r.system_lines})),
                ("basis", r.basis.into()),
                ("from", r.old_name.clone().into()),
            ],
        );
        ctx.idx.insert(r.new_name.clone(), r.participant_id.clone());
    }
    let mut warnings: Vec<String> = parsed.warnings.clone();
    for sp in &suspects {
        let why = if sp.problems.is_empty() { "근거가 부족하다".to_string() } else { sp.problems.join("; ") };
        warnings.push(format!(
            "발신자 이름 변경 의심 {} -> {} (근거 {}건)을 확정하지 않았다: {}. 맞다면 --accept-rename '{}={}' 로 확인하라",
            repr(&sp.old_name),
            repr_opt(sp.new_name.as_deref()),
            sp.matched,
            why,
            sp.old_name,
            sp.new_name.as_deref().unwrap_or("None"),
        ));
    }

    let renames: Vec<(String, String)> = confirmed.iter().map(|r| (r.old_name.clone(), r.new_name.clone())).collect();
    let old_key = |rec: &Record| -> Key {
        if rec.kind == "system" {
            // 폴드 전이라 기록에는 아직 옛 이름이 남아 있다
            let mut t = rec.text.clone();
            for (o, n) in &renames {
                t = relabel_text(&t, o, n);
            }
            return vec!["s".into(), rec.date.clone(), t];
        }
        key_of(rec)
    };

    let reg = &state.registry;
    let old_ids: Vec<String> = state.visible.clone();
    let old_keys: Vec<Key> = old_ids.iter().map(|i| old_key(&reg[i])).collect();
    let new_keys: Vec<Key> = new.iter().map(|e| entry_key(e, &ctx.idx)).collect();
    let mut assigned: Vec<Option<String>> = vec![None; new.len()];
    let mut vanished: Vec<String> = Vec::new();

    for (tag, i1, i2, j1, j2) in SequenceMatcher::new(&old_keys, &new_keys).get_opcodes() {
        if tag == Tag::Equal {
            for k in 0..(i2 - i1) {
                assigned[j1 + k] = Some(old_ids[i1 + k].clone());
            }
            continue;
        }
        let news: Vec<usize> = (j1..j2).collect();
        ctx.block(&old_ids[i1..i2], &news, new, &mut assigned, &mut vanished);
    }

    // 사라진 항목
    for oid in &vanished {
        let covered = parsed.first_date.as_deref().map_or(false, |fd| !fd.is_empty() && reg[oid].date.as_str() >= fd);
        ctx.emit(
            "message.missing",
            vec![("message_id", oid.clone().into()), ("range_covered", covered.into()), ("basis", "absent_in_export".into())],
        );
    }

    let n_active = old_ids.iter().filter(|i| reg[*i].kind == "message" && reg[*i].status == "active").count();
    let n_lost = vanished.iter().filter(|i| reg[*i].kind == "message" && reg[*i].status == "active").count();
    if !force && n_active >= 4 && n_lost * 2 > n_active {
        return Err(KktError::new(
            "mass_loss",
            format!(
                "활성 메시지 {n_active}개 중 {n_lost}개가 사라졌다. 이 PC의 대화 내역이 지워졌거나 내보내기 범위가 달라졌을 수 있다. 확인 후 --force로 다시 실행."
            ),
        ));
    }

    let mut events = ctx.events;
    let visible: Vec<Value> = assigned.into_iter().map(|a| a.map(Value::from).unwrap_or(Value::Null)).collect();
    events.push(json!({
        "type": "export.ingested", "conversation_id": conversation_id,
        "observed_at": observed_at, "export_sha256": export_sha256,
        "export_name": export_name, "saved_at": parsed.saved_at,
        "first_date": parsed.first_date, "last_date": parsed.last_date,
        "visible": visible, "warnings": warnings,
    }));
    Ok(events)
}
