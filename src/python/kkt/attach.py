"""카카오톡에서 저장한 사진·동영상을 아카이브에 들이고 해당 메시지와 연결한다.

연결 근거: 저장된 파일명 KakaoTalk_YYYYMMDD_HHMMSSmmm.ext 의 시각(초, 밀리초 포함)과
TXT의 '[오전 9:28] 사진' 의 분 단위 시각. 같은 날짜/분 안에서 사진 수(메시지 수가 아니라 '사진 3장' 같은 줄은 3장)와
파일 수가 같을 때만 메시지 순서, 시각 순서대로 짝짓는다. 개수가 다르면 추측하지 않고 연결하지 않는다.
동영상은 `동영상` 메시지(1개)와 `.mp4` 파일을 같은 규칙으로 짝짓는다. 사진과 동영상은 서로 섞이지 않는다.

주의: 카카오톡 기본 저장 폴더에는 사용자의 다른 파일이 섞여 있다. 이 모듈은 파일명이
KakaoTalk_날짜_시각 패턴인 것만 읽고, 원본은 수정하지 않는다 (복사만 한다).
"""
from __future__ import annotations

import hashlib
import re
import shutil
from collections import defaultdict
from pathlib import Path

from .archive import Archive

_NAME_RE = re.compile(
    r"^KakaoTalk_(?P<y>\d{4})(?P<mo>\d{2})(?P<d>\d{2})_(?P<h>\d{2})(?P<mi>\d{2})(?P<s>\d{2})(?P<ms>\d{3})?"
    r"(?:_\d{2})?(?: \(\d+\))?\.(?P<ext>jpg|jpeg|png|gif|webp|bmp|mp4)$", re.IGNORECASE)
# '사진 N장' 묶음을 저장하면 첫 장은 KakaoTalk_날짜_시각.jpg, 나머지는 같은 시각에 _01, _02 … 가 붙는다 (실측).
# 한 묶음의 사진은 taken_at 이 모두 같으므로, 같은 시각 안의 순서는 이름순(접미사 없는 것 먼저, _01, _02 …)이다.
_MIME = {"jpg": "image/jpeg", "jpeg": "image/jpeg", "png": "image/png", "gif": "image/gif",
         "webp": "image/webp", "bmp": "image/bmp", "mp4": "video/mp4"}
_VIDEO_EXTS = {"mp4"}                      # 실측: 동영상은 KakaoTalk_날짜_시각.mp4 로 저장된다


def kind_of_ext(ext: str) -> str:
    return "video" if ext.lower() in _VIDEO_EXTS else "image"


def kind_of_mime(mime: str) -> str:
    return "video" if (mime or "").startswith("video/") else "image"


def sha256_file(path: Path) -> tuple[str, int]:
    """(sha256, 크기). 동영상은 클 수 있어 조각 단위로 읽는다."""
    h, n = hashlib.sha256(), 0
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
            n += len(chunk)
    return h.hexdigest(), n


_DUP_SUFFIX_RE = re.compile(r" \(\d+\)(?=\.[^.]+$)")


def base_name(name: str) -> str:
    """' (1)' 같은 재저장 접미사를 뗀 이름. 같은 사진을 다시 저장하면 이 접미사만 붙는다."""
    return _DUP_SUFFIX_RE.sub("", name)


def scan(src_dir: Path) -> tuple[list, list]:
    """(저장 대상 목록, 건너뛴 파일명). 동영상도 대상에 넣는다."""
    files, skipped, _ = _scan(src_dir, True)
    return files, skipped


def _scan(src_dir: Path, videos: bool) -> tuple[list, list, list]:
    """(저장 대상 목록, 건너뛴 파일명, 동영상 제외로 뺀 파일명).

    같은 사진을 다시 저장한 사본(' (1)' 등, 이름이 같고 내용이 같은 것)만 하나로 묶고 이름은 aliases로 둔다.
    이름(시각)이 다르면 내용이 같아도 별개다: 같은 사진을 다른 메시지로 다시 보냈을 수 있다 (실측: 같은 사진을
    여러 묶음에 올림). 이때 보관본(blob)은 해시로 하나만 두고 첨부 기록만 따로 둔다.
    """
    by_key: dict = {}
    skipped: list = []
    excluded: list = []
    # ' (1)' 같은 중복 저장본이 대표 이름이 되지 않도록, 접미사 없는 이름을 먼저 본다
    for p in sorted(Path(src_dir).iterdir(), key=lambda q: (" (" in q.name, q.name)):
        m = _NAME_RE.match(p.name)
        if not p.is_file() or not m:
            skipped.append(p.name)
            continue
        if not videos and kind_of_ext(m["ext"]) == "video":
            excluded.append(p.name)                  # 카카오톡 파일이지만 동영상 보관을 끈 경우
            continue
        sha, size = sha256_file(p)
        ms = m["ms"] or "000"
        taken = f"{m['y']}-{m['mo']}-{m['d']}T{m['h']}:{m['mi']}:{m['s']}.{ms}+09:00"
        key = (sha, base_name(p.name))
        if key in by_key:
            by_key[key]["aliases"].append(p.name)
            continue
        by_key[key] = {
            "sha256": sha, "filename": p.name, "aliases": [], "size": size,
            "mime_type": _MIME[m["ext"].lower()], "ext": m["ext"].lower(), "kind": kind_of_ext(m["ext"]),
            "taken_at": taken, "date": f"{m['y']}-{m['mo']}-{m['d']}",
            "hhmm": f"{m['h']}:{m['mi']}", "src": p,
        }
    return sorted(by_key.values(), key=lambda f: f["taken_at"]), skipped, excluded


def ingest_attachments(arch: Archive, src_dir: Path, videos: bool = True) -> dict:
    state, valid = arch.load()
    files, skipped, excluded = _scan(src_dir, videos)
    events: list = []
    base = {"conversation_id": arch.conversation_id}

    # 이미 보관한 첨부인지는 (내용, 재저장 접미사를 뗀 이름) 으로 본다. 같은 사진을 다른 시각에 다시 보낸 것은 새 첨부다.
    known = {(a["sha256"], base_name(n)) for a in state.attachments.values() for n in [a["filename"], *a["aliases"]]}
    used_ids = set(state.attachments)
    new_files = []
    for f in files:
        if (f["sha256"], base_name(f["filename"])) in known:
            continue
        aid, n = f"att_{f['sha256'][:16]}", 2
        while aid in used_ids:                       # 같은 내용의 두 번째 첨부부터는 _2, _3 …
            aid, n = f"att_{f['sha256'][:16]}_{n}", n + 1
        used_ids.add(aid)
        f["attachment_id"] = aid
        new_files.append(f)
    for f in new_files:
        key = f"attachments/{f['kind']}/{f['sha256'][:2]}/{f['sha256']}.{f['ext']}"
        dest = arch.dir / key
        dest.parent.mkdir(parents=True, exist_ok=True)
        if not dest.exists():
            shutil.copy2(f["src"], dest)
        if sha256_file(dest)[0] != f["sha256"]:
            raise RuntimeError(f"복사본의 해시가 다르다: {dest}")
        events.append({**base, "type": "attachment.saved", "attachment_id": f["attachment_id"],
                       "sha256": f["sha256"], "filename": f["filename"], "aliases": f["aliases"],
                       "size": f["size"], "mime_type": f["mime_type"], "storage_key": key,
                       "taken_at": f["taken_at"], "observed_at": f["taken_at"]})

    # 연결: 같은 날짜/분 그룹에서 개수가 같을 때만
    all_att = {**state.attachments}
    for f in new_files:
        all_att[f["attachment_id"]] = {"attachment_id": f["attachment_id"], "taken_at": f["taken_at"],
                                       "date": f["date"], "hhmm": f["hhmm"], "mime_type": f["mime_type"]}
    linked_att = {a for r in state.registry.values() for a in r["attachment_ids"]}
    msgs, atts = defaultdict(list), defaultdict(list)
    for r in state.registry.values():
        if r["kind"] == "message" and r["content_type"] in ("image", "video") and not r["attachment_ids"]:
            if r["content_type"] == "video" and not videos:
                continue
            msgs[(r["content_type"], r["date"], r["hhmm"])].append((r["id"], r["image_count"]))
    for a in all_att.values():
        if a["attachment_id"] not in linked_att:
            d, t = a["taken_at"][:10], a["taken_at"][11:16]
            atts[(kind_of_mime(a.get("mime_type")), d, t)].append(a)
    unmatched = []
    for grp, mlist in msgs.items():
        kind = grp[0]
        alist = sorted(atts.get(grp, []), key=lambda a: a["taken_at"])
        want = sum(n for _, n in mlist)                  # '사진 3장' 한 줄은 사진 3장
        if want == len(alist):
            it = iter(alist)
            for mid, n in mlist:
                for _ in range(n):
                    a = next(it)
                    events.append({**base, "type": "attachment.linked", "message_id": mid,
                                   "attachment_id": a["attachment_id"], "basis": "minute_match_ordered",
                                   "observed_at": a["taken_at"]})
        else:
            u = {"minute": f"{grp[1]} {grp[2]}", ("image_messages" if kind == "image" else "video_messages"): len(mlist),
                 "files": len(alist)}
            if want != len(mlist):
                u["photos"] = want
            unmatched.append(u)

    if events:
        events.append({**base, "type": "attach.committed", "observed_at": events[0]["observed_at"]})
        arch.commit(state, valid, events)
    out = {"saved": len(new_files), "linked": sum(e["type"] == "attachment.linked" for e in events),
           "skipped_non_kakao_files": len(skipped), "unmatched_groups": unmatched}
    n_videos = sum(f["kind"] == "video" for f in new_files)
    if n_videos:
        out["saved_videos"] = n_videos                  # 동영상이 있을 때만 나타나는 키 (기존 출력은 그대로)
    if excluded:
        out["videos_excluded"] = len(excluded)
    return out
