"""카카오톡에서 저장한 사진을 아카이브에 들이고 사진 메시지와 연결한다.

연결 근거: 저장된 파일명 KakaoTalk_YYYYMMDD_HHMMSSmmm.ext 의 시각(초, 밀리초 포함)과
TXT의 '[오전 9:28] 사진' 의 분 단위 시각. 같은 날짜/분 안에서 사진 메시지 수와 파일 수가
같을 때만 시각 순서대로 짝짓는다. 개수가 다르면 추측하지 않고 연결하지 않는다.

주의: 카카오톡 기본 저장 폴더에는 사용자의 다른 파일이 섞여 있다. 이 모듈은 파일명이
KakaoTalk_날짜_시각 패턴인 것만 읽고, 원본은 수정하지 않는다 (복사만 한다).
"""
from __future__ import annotations

import re
import shutil
from collections import defaultdict
from pathlib import Path

from .archive import Archive, sha256_bytes

_NAME_RE = re.compile(
    r"^KakaoTalk_(?P<y>\d{4})(?P<mo>\d{2})(?P<d>\d{2})_(?P<h>\d{2})(?P<mi>\d{2})(?P<s>\d{2})(?P<ms>\d{3})?"
    r"(?: \(\d+\))?\.(?P<ext>jpg|jpeg|png|gif|webp|bmp)$", re.IGNORECASE)
_MIME = {"jpg": "image/jpeg", "jpeg": "image/jpeg", "png": "image/png", "gif": "image/gif",
         "webp": "image/webp", "bmp": "image/bmp"}


def scan(src_dir: Path) -> tuple[list, list]:
    """(저장 대상 목록, 건너뛴 파일명). 같은 내용(sha256)은 하나로 묶고 이름은 aliases로 둔다."""
    by_sha: dict = {}
    skipped: list = []
    # ' (1)' 같은 중복 저장본이 대표 이름이 되지 않도록, 접미사 없는 이름을 먼저 본다
    for p in sorted(Path(src_dir).iterdir(), key=lambda q: (" (" in q.name, q.name)):
        m = _NAME_RE.match(p.name)
        if not p.is_file() or not m:
            skipped.append(p.name)
            continue
        data = p.read_bytes()
        sha = sha256_bytes(data)
        ms = m["ms"] or "000"
        taken = f"{m['y']}-{m['mo']}-{m['d']}T{m['h']}:{m['mi']}:{m['s']}.{ms}+09:00"
        if sha in by_sha:
            by_sha[sha]["aliases"].append(p.name)
            continue
        by_sha[sha] = {
            "sha256": sha, "filename": p.name, "aliases": [], "size": len(data),
            "mime_type": _MIME[m["ext"].lower()], "ext": m["ext"].lower(),
            "taken_at": taken, "date": f"{m['y']}-{m['mo']}-{m['d']}",
            "hhmm": f"{m['h']}:{m['mi']}", "src": p,
        }
    return sorted(by_sha.values(), key=lambda f: f["taken_at"]), skipped


def ingest_attachments(arch: Archive, src_dir: Path) -> dict:
    state, valid = arch.load()
    files, skipped = scan(src_dir)
    events: list = []
    base = {"conversation_id": arch.conversation_id}

    new_files = [f for f in files if f"att_{f['sha256'][:16]}" not in state.attachments]
    for f in new_files:
        key = f"attachments/image/{f['sha256'][:2]}/{f['sha256']}.{f['ext']}"
        dest = arch.dir / key
        dest.parent.mkdir(parents=True, exist_ok=True)
        if not dest.exists():
            shutil.copy2(f["src"], dest)
        if sha256_bytes(dest.read_bytes()) != f["sha256"]:
            raise RuntimeError(f"복사본의 해시가 다르다: {dest}")
        f["attachment_id"] = f"att_{f['sha256'][:16]}"
        events.append({**base, "type": "attachment.saved", "attachment_id": f["attachment_id"],
                       "sha256": f["sha256"], "filename": f["filename"], "aliases": f["aliases"],
                       "size": f["size"], "mime_type": f["mime_type"], "storage_key": key,
                       "taken_at": f["taken_at"], "observed_at": f["taken_at"]})

    # 연결: 같은 날짜/분 그룹에서 개수가 같을 때만
    all_att = {**state.attachments}
    for f in new_files:
        all_att[f["attachment_id"]] = {"attachment_id": f["attachment_id"], "taken_at": f["taken_at"],
                                       "date": f["date"], "hhmm": f["hhmm"]}
    linked_att = {r["attachment_id"] for r in state.registry.values() if r["attachment_id"]}
    msgs, atts = defaultdict(list), defaultdict(list)
    for r in state.registry.values():
        if r["kind"] == "message" and r["content_type"] == "image" and not r["attachment_id"]:
            msgs[(r["date"], r["hhmm"])].append(r["id"])
    for a in all_att.values():
        if a["attachment_id"] not in linked_att:
            d, t = a["taken_at"][:10], a["taken_at"][11:16]
            atts[(d, t)].append(a)
    unmatched = []
    for grp, mids in msgs.items():
        alist = sorted(atts.get(grp, []), key=lambda a: a["taken_at"])
        if len(mids) == len(alist):
            for mid, a in zip(mids, alist):
                events.append({**base, "type": "attachment.linked", "message_id": mid,
                               "attachment_id": a["attachment_id"], "basis": "minute_match_ordered",
                               "observed_at": a["taken_at"]})
        else:
            unmatched.append({"minute": f"{grp[0]} {grp[1]}", "image_messages": len(mids),
                              "files": len(alist)})

    if events:
        events.append({**base, "type": "attach.committed", "observed_at": events[0]["observed_at"]})
        arch.commit(state, valid, events)
    return {"saved": len(new_files), "linked": sum(e["type"] == "attachment.linked" for e in events),
            "skipped_non_kakao_files": len(skipped), "unmatched_groups": unmatched}
