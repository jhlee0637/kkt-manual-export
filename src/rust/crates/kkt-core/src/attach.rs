//! 카카오톡에서 저장한 사진·동영상을 아카이브에 들이고 해당 메시지와 연결한다 (src/python/kkt/attach.py 의 이식).
//!
//! 연결 근거: 저장된 파일명 KakaoTalk_YYYYMMDD_HHMMSSmmm.ext 의 시각(초, 밀리초 포함)과 TXT 의 '[오전 9:28] 사진' 의 분 단위 시각.
//! 같은 날짜/분 안에서 사진 수(`사진 3장` 한 줄은 3장)와 파일 수가 같을 때만 메시지 순서, 시각 순서대로 짝짓는다.
//! 개수가 다르면 추측하지 않고 연결하지 않는다. 동영상은 `동영상` 메시지(1개)와 `.mp4` 파일을 같은 규칙으로 짝짓고, 사진과 섞지 않는다.
//!
//! 주의: 카카오톡 기본 저장 폴더에는 사용자의 다른 파일이 섞여 있다. 파일명이 KakaoTalk_날짜_시각 패턴인 것만 읽고,
//! 원본은 수정하지 않는다 (복사만 한다).

use crate::archive::Archive;
use crate::error::{KktError, Result};
use indexmap::IndexMap;
use regex_lite::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// '사진 N장' 묶음을 저장하면 첫 장은 KakaoTalk_날짜_시각.jpg, 나머지는 같은 시각에 _01, _02 … 가 붙는다 (실측).
/// 한 묶음의 사진은 taken_at 이 모두 같으므로, 같은 시각 안의 순서는 이름순(접미사 없는 것 먼저, _01, _02 …)이다.
fn name_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^KakaoTalk_(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})(\d{3})?(?:_\d{2})?(?: \(\d+\))?\.(jpg|jpeg|png|gif|webp|bmp|mp4)$").unwrap()
    })
}

/// 동영상으로 보는 확장자 (실측: 동영상은 KakaoTalk_날짜_시각.mp4 로 저장된다).
fn kind_of_ext(ext: &str) -> &'static str {
    if ext == "mp4" { "video" } else { "image" }
}

fn kind_of_mime(mime: &str) -> &'static str {
    if mime.starts_with("video/") { "video" } else { "image" }
}

/// (sha256, 크기). 동영상은 클 수 있어 조각 단위로 읽는다.
fn sha256_file(path: &Path) -> Result<(String, usize)> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0usize;
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k;
    }
    Ok((h.finalize().iter().map(|x| format!("{x:02x}")).collect(), n))
}

fn mime(ext: &str) -> &'static str {
    match ext {
        "mp4" => "video/mp4",
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
    pub kind: &'static str,
    pub taken_at: String,
    pub src: PathBuf,
    pub attachment_id: String,
}

/// ' (1)' 같은 재저장 접미사를 뗀 이름. 같은 사진을 다시 저장하면 이 접미사만 붙는다.
pub fn base_name(name: &str) -> String {
    if let Some(dot) = name.rfind('.') {
        let (stem, ext) = name.split_at(dot);
        if ext.len() > 1 && stem.ends_with(')') {
            if let Some(open) = stem.rfind(" (") {
                let digits = &stem[open + 2..stem.len() - 1];
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                    return format!("{}{}", &stem[..open], ext);
                }
            }
        }
    }
    name.to_string()
}

/// (저장 대상 목록, 건너뛴 파일 수). 같은 사진을 다시 저장한 사본(' (1)' 등, 이름이 같고 내용이 같은 것)만 하나로 묶고
/// 이름은 aliases 로 둔다. 이름(시각)이 다르면 내용이 같아도 별개다 (같은 사진을 다른 메시지로 다시 보냈을 수 있다).
pub fn scan(src_dir: &Path) -> Result<(Vec<Found>, usize)> {
    let (files, skipped, _) = scan_inner(src_dir, true)?;
    Ok((files, skipped))
}

/// `(저장 대상 목록, 건너뛴 파일 수, 동영상 제외로 뺀 파일 수)`. `videos` 가 false 면 `.mp4` 는 대상에서 뺀다.
fn scan_inner(src_dir: &Path, videos: bool) -> Result<(Vec<Found>, usize, usize)> {
    let mut by_key: IndexMap<String, Found> = IndexMap::new();
    let mut skipped = 0usize;
    let mut excluded = 0usize;
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
        if !videos && kind_of_ext(&caps[8].to_lowercase()) == "video" {
            excluded += 1; // 카카오톡 파일이지만 동영상 보관을 끈 경우
            continue;
        }
        let (sha, size) = sha256_file(&path)?;
        let ms = caps.get(7).map_or("000", |m| m.as_str());
        let taken = format!("{}-{}-{}T{}:{}:{}.{}+09:00", &caps[1], &caps[2], &caps[3], &caps[4], &caps[5], &caps[6], ms);
        let key = format!("{sha}\u{0}{}", base_name(&name));
        if let Some(f) = by_key.get_mut(&key) {
            f.aliases.push(name);
            continue;
        }
        let ext = caps[8].to_lowercase();
        by_key.insert(
            key,
            Found {
                attachment_id: String::new(),
                sha256: sha,
                filename: name,
                aliases: Vec::new(),
                size,
                mime_type: mime(&ext),
                kind: kind_of_ext(&ext),
                ext,
                taken_at: taken,
                src: path,
            },
        );
    }
    let mut files: Vec<Found> = by_key.into_values().collect();
    files.sort_by(|a, b| a.taken_at.cmp(&b.taken_at)); // 안정 정렬
    Ok((files, skipped, excluded))
}

pub fn ingest_attachments(arch: &Archive, src_dir: &Path, videos: bool) -> Result<Value> {
    let (mut st, valid) = arch.load()?;
    let (files, skipped, excluded) = scan_inner(src_dir, videos)?;
    let mut events: Vec<Value> = Vec::new();
    let conv = &arch.conversation_id;

    // 이미 보관한 첨부인지는 (내용, 재저장 접미사를 뗀 이름) 으로 본다. 같은 사진을 다른 시각에 다시 보낸 것은 새 첨부다.
    let known: std::collections::HashSet<(String, String)> = st
        .attachments
        .values()
        .flat_map(|a| std::iter::once(&a.filename).chain(a.aliases.iter()).map(move |n| (a.sha256.clone(), base_name(n))))
        .collect();
    let mut used_ids: std::collections::HashSet<String> = st.attachments.keys().cloned().collect();
    let mut new_files: Vec<Found> = Vec::new();
    for mut f in files {
        if known.contains(&(f.sha256.clone(), base_name(&f.filename))) {
            continue;
        }
        let mut aid = format!("att_{}", &f.sha256[..16]);
        let mut n = 2;
        while used_ids.contains(&aid) {
            // 같은 내용의 두 번째 첨부부터는 _2, _3 …
            aid = format!("att_{}_{n}", &f.sha256[..16]);
            n += 1;
        }
        used_ids.insert(aid.clone());
        f.attachment_id = aid;
        new_files.push(f);
    }
    for f in &new_files {
        let key = format!("attachments/{}/{}/{}.{}", f.kind, &f.sha256[..2], f.sha256, f.ext);
        let dest = arch.dir.join(&key);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        if !dest.exists() {
            fs::copy(&f.src, &dest)?;
        }
        if sha256_file(&dest)?.0 != f.sha256 {
            return Err(KktError::new("io_error", format!("복사본의 해시가 다르다: {}", dest.display())));
        }
        events.push(json!({
            "conversation_id": conv, "type": "attachment.saved", "attachment_id": f.attachment_id,
            "sha256": f.sha256, "filename": f.filename, "aliases": f.aliases, "size": f.size,
            "mime_type": f.mime_type, "storage_key": key, "taken_at": f.taken_at, "observed_at": f.taken_at,
        }));
    }

    // 연결: 같은 날짜/분 그룹에서 개수가 같을 때만
    // (taken_at, 종류) — 사진과 동영상은 서로 짝지어지지 않는다
    let mut all_att: IndexMap<String, (String, &'static str)> =
        st.attachments.iter().map(|(k, a)| (k.clone(), (a.taken_at.clone(), kind_of_mime(&a.mime_type)))).collect();
    for f in &new_files {
        all_att.insert(f.attachment_id.clone(), (f.taken_at.clone(), f.kind));
    }
    let linked: std::collections::HashSet<String> =
        st.registry.values().flat_map(|r| r.attachment_ids.iter().cloned()).collect();
    type Group = (&'static str, String, String); // (종류, 날짜, 분)
    let mut msgs: IndexMap<Group, Vec<(String, u64)>> = IndexMap::new();
    for r in st.registry.values() {
        if r.kind == "message" && (r.content_type == "image" || r.content_type == "video") && r.attachment_ids.is_empty() {
            if r.content_type == "video" && !videos {
                continue;
            }
            let kind = if r.content_type == "video" { "video" } else { "image" };
            msgs.entry((kind, r.date.clone(), r.hhmm.clone().unwrap_or_default())).or_default().push((r.id.clone(), r.image_count));
        }
    }
    let mut atts: IndexMap<Group, Vec<(String, String)>> = IndexMap::new();
    for (aid, (taken, kind)) in &all_att {
        if !linked.contains(aid) {
            let g: Group = (*kind, taken[..10].to_string(), taken[11..16].to_string());
            atts.entry(g).or_default().push((aid.clone(), taken.clone()));
        }
    }
    let mut unmatched: Vec<Value> = Vec::new();
    for (grp, mids) in &msgs {
        let mut alist = atts.get(grp).cloned().unwrap_or_default();
        alist.sort_by(|a, b| a.1.cmp(&b.1)); // 안정 정렬
        let want: u64 = mids.iter().map(|(_, n)| n).sum(); // '사진 3장' 한 줄은 사진 3장
        if want == alist.len() as u64 {
            let mut it = alist.iter();
            for (mid, n) in mids {
                for _ in 0..*n {
                    let (aid, taken) = it.next().expect("개수가 같음을 확인했다");
                    events.push(json!({
                        "conversation_id": conv, "type": "attachment.linked", "message_id": mid,
                        "attachment_id": aid, "basis": "minute_match_ordered", "observed_at": taken,
                    }));
                }
            }
        } else {
            let count_key = if grp.0 == "image" { "image_messages" } else { "video_messages" };
            let mut u = json!({"minute": format!("{} {}", grp.1, grp.2), count_key: mids.len(), "files": alist.len()});
            if want != mids.len() as u64 {
                u["photos"] = want.into();
            }
            unmatched.push(u);
        }
    }

    let linked_count = events.iter().filter(|e| e["type"] == "attachment.linked").count();
    if !events.is_empty() {
        let first_obs = events[0]["observed_at"].clone();
        events.push(json!({"conversation_id": conv, "type": "attach.committed", "observed_at": first_obs}));
        arch.commit(&mut st, valid, &mut events)?;
    }
    let mut out = json!({"saved": new_files.len(), "linked": linked_count, "skipped_non_kakao_files": skipped, "unmatched_groups": unmatched});
    let n_videos = new_files.iter().filter(|f| f.kind == "video").count();
    if n_videos > 0 {
        out["saved_videos"] = n_videos.into(); // 동영상이 있을 때만 나타나는 키 (기존 출력은 그대로)
    }
    if excluded > 0 {
        out["videos_excluded"] = excluded.into();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_name_strips_only_the_resave_suffix() {
        assert_eq!(base_name("KakaoTalk_20261006_080735851 (1).jpg"), "KakaoTalk_20261006_080735851.jpg");
        assert_eq!(base_name("KakaoTalk_20261006_080735851_01 (12).jpg"), "KakaoTalk_20261006_080735851_01.jpg");
        assert_eq!(base_name("KakaoTalk_20261006_080735851.jpg"), "KakaoTalk_20261006_080735851.jpg");
        assert_eq!(base_name("a (x).jpg"), "a (x).jpg");
        assert_eq!(base_name("a ().jpg"), "a ().jpg");
        assert_eq!(base_name("a (1)"), "a (1)");
    }
}
