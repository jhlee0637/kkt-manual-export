"""아카이브 디렉터리 하나 = 대화방 하나.

    archive/<conversation_id>/
        events.jsonl        추가 전용 이벤트 로그 (유일한 원본)
        raw/                반영한 내보내기 TXT 원본 (삭제하지 않는다)
        attachments/image/<sha256 앞 2자>/<sha256>.<ext>
"""
from __future__ import annotations

import hashlib
import shutil
from collections import Counter
from pathlib import Path

from . import SCHEMA_VERSION, state as st
from .parse import ParsedExport, parse_export
from .reconcile import IngestError, reconcile


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def decode_export(b: bytes) -> str:
    for enc in ("utf-8-sig", "cp949"):
        try:
            return b.decode(enc)
        except UnicodeDecodeError:
            continue
    raise IngestError("내보내기 파일의 인코딩을 알 수 없다 (utf-8, cp949 둘 다 실패)")


def default_conversation_id(title: str | None) -> str:
    # 방 이름은 바뀔 수 있다. 임시 기본값일 뿐이며 --conversation으로 고정하는 편이 안전하다.
    return "kt_" + hashlib.sha1((title or "unknown").encode("utf-8")).hexdigest()[:10]


class Archive:
    def __init__(self, root: Path, conversation_id: str):
        self.conversation_id = conversation_id
        self.dir = Path(root) / conversation_id
        self.events_path = self.dir / "events.jsonl"
        self.raw_dir = self.dir / "raw"
        self.attach_dir = self.dir / "attachments" / "image"

    def load(self):
        return st.load(self.events_path)

    def commit(self, state: st.State, valid_lines: int, events: list) -> int:
        for ev in events:
            state.seq += 1
            ev["event_id"] = f"ev_{state.seq:06d}"
            ev["schema_version"] = SCHEMA_VERSION
        for ev in events:
            state.apply(ev)
        st.append(self.events_path, valid_lines, events)
        return valid_lines + len(events)

    def ingest(self, path: Path, *, parsed: ParsedExport | None = None, force: bool = False,
               accept_renames: dict | None = None, link: dict | None = None) -> dict:
        data = Path(path).read_bytes()
        sha = sha256_bytes(data)
        state, valid = self.load()
        if state.known_export(sha):
            return {"export": Path(path).name, "skipped": "이미 반영된 내보내기", "events": {}}
        parsed = parsed or parse_export(decode_export(data))
        events = reconcile(state, parsed, conversation_id=self.conversation_id,
                           export_name=Path(path).name, export_sha256=sha, force=force,
                           accept_renames=accept_renames, link=link)
        self.raw_dir.mkdir(parents=True, exist_ok=True)
        dest = self.raw_dir / Path(path).name
        if dest.exists() and sha256_bytes(dest.read_bytes()) != sha:
            dest = self.raw_dir / f"{sha[:8]}_{Path(path).name}"
        if not dest.exists():
            shutil.copy2(path, dest)
        counts = Counter(e["type"] for e in events if e["type"] != "export.ingested")
        self.commit(state, valid, events)
        warns = next(e["warnings"] for e in events if e["type"] == "export.ingested")
        return {"export": Path(path).name, "events": dict(counts), "warnings": warns}

    def link_participants(self, keep_name: str, merge_name: str) -> dict:
        """사람이 확인한 연결: merge_name 의 참가자를 keep_name 의 참가자로 합친다.
        새 이름이 새 메시지에만 나타나는 경우(과거 메시지는 옛 이름 유지)처럼 TXT 에 근거가 없을 때 쓴다.
        합친 뒤의 현재 이름은 merge_name(더 최근 이름으로 간주)이 된다."""
        state, valid = self.load()
        by_name = {p["current"]: p["id"] for p in state.participants.values()}
        for n in (keep_name, merge_name):
            if n not in by_name:
                raise IngestError(f"현재 이름이 {n!r}인 참가자가 없다: {sorted(by_name)}")
        if keep_name == merge_name:
            raise IngestError("같은 참가자를 합칠 수 없다")
        base = {"conversation_id": self.conversation_id,
                "observed_at": (state.ingests[-1]["saved_at"] if state.ingests else None)}
        events = [{**base, "type": "participant.linked", "participant_id": by_name[keep_name],
                   "merged_participant_id": by_name[merge_name], "current_name": merge_name,
                   "basis": "manual"},
                  {**base, "type": "state.committed"}]
        self.commit(state, valid, events)
        return {"kept": by_name[keep_name], "merged": by_name[merge_name], "current_name": merge_name}
