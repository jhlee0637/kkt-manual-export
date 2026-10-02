//! 아카이브 디렉터리 하나 = 대화방 하나 (kkt/archive.py 의 이식).
//!
//! ```text
//! archive/<conversation_id>/
//!     events.jsonl        추가 전용 이벤트 로그 (유일한 원본)
//!     raw/                반영한 내보내기 TXT 원본 (삭제하지 않는다)
//!     attachments/image/<sha256 앞 2자>/<sha256>.<ext>
//! ```

use crate::error::{KktError, Result};
use crate::parse::{parse_export, ParsedExport};
use crate::pyfmt::repr;
use crate::reconcile::reconcile;
use crate::state::{self, State};
use crate::SCHEMA_VERSION;
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub fn sha256_hex(b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b);
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

/// utf-8(BOM 허용) -> cp949 순으로 시도한다. 둘 다 실패하면 bad_encoding.
pub fn decode_export(b: &[u8]) -> Result<String> {
    let body = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b);
    if let Ok(s) = std::str::from_utf8(body) {
        return Ok(s.to_string());
    }
    // WHATWG 의 euc-kr 은 실제로 cp949(통합 완성형)다.
    if let Some(s) = encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(b) {
        return Ok(s.into_owned());
    }
    Err(KktError::new("bad_encoding", "내보내기 파일의 인코딩을 알 수 없다 (utf-8, cp949 둘 다 실패)"))
}

pub struct Archive {
    pub conversation_id: String,
    pub dir: PathBuf,
    pub events_path: PathBuf,
    pub raw_dir: PathBuf,
}

impl Archive {
    pub fn new(root: &Path, conversation_id: &str) -> Self {
        let dir = root.join(conversation_id);
        Archive {
            conversation_id: conversation_id.to_string(),
            events_path: dir.join("events.jsonl"),
            raw_dir: dir.join("raw"),
            dir,
        }
    }

    pub fn load(&self) -> Result<(State, usize)> {
        state::load(&self.events_path)
    }

    /// event_id 와 schema_version 을 붙이고 상태에 적용한 뒤 로그에 추가한다.
    pub fn commit(&self, st: &mut State, valid_lines: usize, events: &mut [Value]) -> Result<usize> {
        for ev in events.iter_mut() {
            st.seq += 1;
            let m = ev.as_object_mut().expect("이벤트는 객체다");
            m.insert("event_id".into(), format!("ev_{:06}", st.seq).into());
            m.insert("schema_version".into(), SCHEMA_VERSION.into());
        }
        for ev in events.iter() {
            st.apply(ev)?;
        }
        state::append(&self.events_path, valid_lines, events)?;
        Ok(valid_lines + events.len())
    }

    pub fn ingest(
        &self,
        path: &Path,
        parsed: Option<&ParsedExport>,
        force: bool,
        accept: &IndexMap<String, String>,
        link: Option<&Value>,
    ) -> Result<Value> {
        let data = fs::read(path)?;
        let sha = sha256_hex(&data);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let (mut st, valid) = self.load()?;
        if st.known_export(&sha) {
            return Ok(json!({"export": name, "skipped": "이미 반영된 내보내기", "events": {}}));
        }
        let owned;
        let parsed = match parsed {
            Some(p) => p,
            None => {
                owned = parse_export(&decode_export(&data)?);
                &owned
            }
        };
        let mut events = reconcile(&st, parsed, &self.conversation_id, &name, &sha, force, accept, link)?;
        fs::create_dir_all(&self.raw_dir)?;
        let mut dest = self.raw_dir.join(&name);
        if dest.exists() && sha256_hex(&fs::read(&dest)?) != sha {
            dest = self.raw_dir.join(format!("{}_{}", &sha[..8], name));
        }
        if !dest.exists() {
            fs::copy(path, &dest)?;
        }
        let mut counts: IndexMap<String, u64> = IndexMap::new();
        for e in &events {
            let t = e["type"].as_str().unwrap_or("");
            if t != "export.ingested" {
                *counts.entry(t.to_string()).or_insert(0) += 1;
            }
        }
        let warns = events.iter().find(|e| e["type"] == "export.ingested").map(|e| e["warnings"].clone()).unwrap_or_else(|| json!([]));
        self.commit(&mut st, valid, &mut events)?;
        let counts_v: Map<String, Value> = counts.into_iter().map(|(k, v)| (k, Value::from(v))).collect();
        Ok(json!({"export": name, "events": counts_v, "warnings": warns}))
    }

    /// 사람이 확인한 연결: merge 이름의 참가자를 keep 이름의 참가자로 합친다. 합친 뒤의 현재 이름은 merge(더 최근 이름)가 된다.
    pub fn link_participants(&self, keep_name: &str, merge_name: &str) -> Result<Value> {
        let (mut st, valid) = self.load()?;
        let mut by_name: IndexMap<String, String> = IndexMap::new();
        for p in st.participants.values() {
            by_name.insert(p.current.clone(), p.id.clone());
        }
        for n in [keep_name, merge_name] {
            if !by_name.contains_key(n) {
                let mut names: Vec<&String> = by_name.keys().collect();
                names.sort();
                return Err(KktError::new(
                    "participant_not_found",
                    format!("현재 이름이 {}인 참가자가 없다: [{}]", repr(n), names.iter().map(|s| repr(s)).collect::<Vec<_>>().join(", ")),
                ));
            }
        }
        if keep_name == merge_name {
            return Err(KktError::new("participant_same", "같은 참가자를 합칠 수 없다"));
        }
        let observed_at: Value = st.ingests.last().and_then(|i| i.saved_at.clone()).map(Value::from).unwrap_or(Value::Null);
        let (keep_id, merge_id) = (by_name[keep_name].clone(), by_name[merge_name].clone());
        let mut events = vec![
            json!({"conversation_id": self.conversation_id, "observed_at": observed_at, "type": "participant.linked",
                   "participant_id": keep_id, "merged_participant_id": merge_id, "current_name": merge_name, "basis": "manual"}),
            json!({"conversation_id": self.conversation_id, "observed_at": observed_at, "type": "state.committed"}),
        ];
        self.commit(&mut st, valid, &mut events)?;
        Ok(json!({"kept": keep_id, "merged": merge_id, "current_name": merge_name}))
    }
}
