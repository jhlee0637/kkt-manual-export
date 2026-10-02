//! 카카오톡에서 저장한 사진을 아카이브에 들이고 사진 메시지와 연결한다 (src/python/kkt/attach.py 의 이식).
//!
//! 연결 근거: 저장된 파일명 KakaoTalk_YYYYMMDD_HHMMSSmmm.ext 의 시각(초, 밀리초 포함)과 TXT 의 '[오전 9:28] 사진' 의 분 단위 시각.
//! 같은 날짜/분 안에서 사진 메시지 수와 파일 수가 같을 때만 시각 순서대로 짝짓는다. 개수가 다르면 추측하지 않고 연결하지 않는다.
//!
//! 주의: 카카오톡 기본 저장 폴더에는 사용자의 다른 파일이 섞여 있다. 파일명이 KakaoTalk_날짜_시각 패턴인 것만 읽고,
//! 원본은 수정하지 않는다 (복사만 한다).

use crate::archive::{sha256_hex, Archive};
use crate::error::{KktError, Result};
use indexmap::IndexMap;
use regex_lite::Regex;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn name_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^KakaoTalk_(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})(\d{3})?(?: \(\d+\))?\.(jpg|jpeg|png|gif|webp|bmp)$").unwrap()
    })
}

fn mime(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/bmp",
    }
}

#[derive(Debug, Clone)]
pub struct Found {
    pub sha256: String,
    pub filename: String,
    pub aliases: Vec<String>,
    pub size: usize,
    pub mime_type: &'static str,
    pub ext: String,
    pub taken_at: String,
    pub src: PathBuf,
    pub attachment_id: String,
}

/// (저장 대상 목록, 건너뛴 파일 수). 같은 내용(sha256)은 하나로 묶고 이름은 aliases 로 둔다.
pub fn scan(src_dir: &Path) -> Result<(Vec<Found>, usize)> {
    let mut by_sha: IndexMap<String, Found> = IndexMap::new();
    let mut skipped = 0usize;
    let mut entries: Vec<(String, PathBuf)> = fs::read_dir(src_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .collect();
    // ' (1)' 같은 중복 저장본이 대표 이름이 되지 않도록, 접미사 없는 이름을 먼저 본다
    entries.sort_by(|a, b| (a.0.contains(" ("), &a.0).cmp(&(b.0.contains(" ("), &b.0)));
    for (name, path) in entries {
        let caps = match name_re().captures(&name) {
            Some(c) if path.is_file() => c,
            _ => {
                skipped += 1;
                continue;
            }
        };
        let data = fs::read(&path)?;
        let sha = sha256_hex(&data);
        let ms = caps.get(7).map_or("000", |m| m.as_str());
        let taken = format!("{}-{}-{}T{}:{}:{}.{}+09:00", &caps[1], &caps[2], &caps[3], &caps[4], &caps[5], &caps[6], ms);
        if let Some(f) = by_sha.get_mut(&sha) {
            f.aliases.push(name);
            continue;
        }
        let ext = caps[8].to_lowercase();
        by_sha.insert(
            sha.clone(),
            Found {
                attachment_id: String::new(),
                sha256: sha,
                filename: name,
                aliases: Vec::new(),
                size: data.len(),
                mime_type: mime(&ext),
                ext,
                taken_at: taken,
                src: path,
            },
        );
    }
    let mut files: Vec<Found> = by_sha.into_values().collect();
    files.sort_by(|a, b| a.taken_at.cmp(&b.taken_at)); // 안정 정렬
    Ok((files, skipped))
}

pub fn ingest_attachments(arch: &Archive, src_dir: &Path) -> Result<Value> {
    let (mut st, valid) = arch.load()?;
    let (files, skipped) = scan(src_dir)?;
    let mut events: Vec<Value> = Vec::new();
    let conv = &arch.conversation_id;

    let new_files: Vec<Found> = files
        .into_iter()
        .map(|mut f| {
            f.attachment_id = format!("att_{}", &f.sha256[..16]);
            f
        })
        .filter(|f| !st.attachments.contains_key(&f.attachment_id))
        .collect();
    for f in &new_files {
        let key = format!("attachments/image/{}/{}.{}", &f.sha256[..2], f.sha256, f.ext);
        let dest = arch.dir.join(&key);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        if !dest.exists() {
            fs::copy(&f.src, &dest)?;
        }
        if sha256_hex(&fs::read(&dest)?) != f.sha256 {
            return Err(KktError::new("io_error", format!("복사본의 해시가 다르다: {}", dest.display())));
        }
        events.push(json!({
            "conversation_id": conv, "type": "attachment.saved", "attachment_id": f.attachment_id,
            "sha256": f.sha256, "filename": f.filename, "aliases": f.aliases, "size": f.size,
            "mime_type": f.mime_type, "storage_key": key, "taken_at": f.taken_at, "observed_at": f.taken_at,
        }));
    }

    // 연결: 같은 날짜/분 그룹에서 개수가 같을 때만
    let mut all_att: IndexMap<String, String> = st.attachments.iter().map(|(k, a)| (k.clone(), a.taken_at.clone())).collect();
    for f in &new_files {
        all_att.insert(f.attachment_id.clone(), f.taken_at.clone());
    }
    let linked: std::collections::HashSet<String> =
        st.registry.values().filter_map(|r| r.attachment_id.clone()).filter(|a| !a.is_empty()).collect();
    let mut msgs: IndexMap<(String, String), Vec<String>> = IndexMap::new();
    for r in st.registry.values() {
        if r.kind == "message" && r.content_type == "image" && r.attachment_id.as_deref().map_or(true, |a| a.is_empty()) {
            msgs.entry((r.date.clone(), r.hhmm.clone().unwrap_or_default())).or_default().push(r.id.clone());
        }
    }
    let mut atts: IndexMap<(String, String), Vec<(String, String)>> = IndexMap::new();
    for (aid, taken) in &all_att {
        if !linked.contains(aid) {
            let (d, t) = (taken[..10].to_string(), taken[11..16].to_string());
            atts.entry((d, t)).or_default().push((aid.clone(), taken.clone()));
        }
    }
    let mut unmatched: Vec<Value> = Vec::new();
    for (grp, mids) in &msgs {
        let mut alist = atts.get(grp).cloned().unwrap_or_default();
        alist.sort_by(|a, b| a.1.cmp(&b.1)); // 안정 정렬
        if mids.len() == alist.len() {
            for (mid, (aid, taken)) in mids.iter().zip(alist.iter()) {
                events.push(json!({
                    "conversation_id": conv, "type": "attachment.linked", "message_id": mid,
                    "attachment_id": aid, "basis": "minute_match_ordered", "observed_at": taken,
                }));
            }
        } else {
            unmatched.push(json!({"minute": format!("{} {}", grp.0, grp.1), "image_messages": mids.len(), "files": alist.len()}));
        }
    }

    let linked_count = events.iter().filter(|e| e["type"] == "attachment.linked").count();
    if !events.is_empty() {
        let first_obs = events[0]["observed_at"].clone();
        events.push(json!({"conversation_id": conv, "type": "attach.committed", "observed_at": first_obs}));
        arch.commit(&mut st, valid, &mut events)?;
    }
    Ok(json!({"saved": new_files.len(), "linked": linked_count, "skipped_non_kakao_files": skipped, "unmatched_groups": unmatched}))
}
