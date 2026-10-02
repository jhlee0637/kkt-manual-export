"""이벤트 로그(events.jsonl)를 접어서(fold) 현재 상태를 만든다.

상태 파일을 따로 두지 않는다. 이벤트 로그가 유일한 원본이고 상태는 항상 로그에서 복원된다.
트랜잭션은 종결 이벤트(export.ingested / attach.committed)로 닫는다.
종결 이벤트 없이 끝난 꼬리(크래시)는 읽을 때 버린다.
"""
from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path

from . import SCHEMA_VERSION

class SchemaError(Exception):
    """이벤트 로그가 현재 코드가 이해하는 버전보다 오래됐다."""


TERMINATORS = {"export.ingested", "attach.committed", "state.committed"}


def relabel_text(text: str, old: str, new: str) -> str:
    """시스템 줄 안의 이름을 치환한다. 시스템 줄에서 이름은 '<이름>님' 꼴로 나온다
    (예: '.님이 A님, B님을 초대했습니다.'). '.' 같은 짧은 이름이 문장부호와 섞이지 않도록 '님'까지 묶어서 바꾼다."""
    return text.replace(f"{old}님", f"{new}님")


def ident_of(rec: dict):
    """발신자의 정체. 이름은 바뀔 수 있으므로 participant_id 를 쓴다 (없으면 이름으로 대신한다)."""
    return rec.get("participant_id") or ("n", rec.get("sender"))


def key_of(rec: dict) -> tuple:
    """내보내기 항목과 기존 기록을 맞추는 키. reconcile.entry_key()와 같은 모양이어야 한다."""
    if rec["kind"] == "message":
        if rec["status"] == "deleted_for_everyone":
            return ("d", rec["date"])
        return ("m", rec["date"], rec["hhmm"], ident_of(rec), rec["text"])
    if rec["kind"] == "deleted_marker":
        return ("d", rec["date"])
    return ("s", rec["date"], rec["text"])


@dataclass
class State:
    registry: dict = field(default_factory=dict)      # message_id -> record (삽입 순서 유지)
    visible: list = field(default_factory=list)       # 마지막 내보내기에 보인 id (순서대로)
    ingests: list = field(default_factory=list)       # 반영한 내보내기 메타
    attachments: dict = field(default_factory=dict)   # attachment_id -> 메타
    participants: dict = field(default_factory=dict)  # participant_id -> {id, names[], current}
    titles: list = field(default_factory=list)        # 방 제목 이력 [{title, observed_at}]
    title: str | None = None                          # 마지막으로 관측한 제목
    seq: int = 0

    def current_names(self) -> dict:
        return {pid: p["current"] for pid, p in self.participants.items()}

    def name_index(self) -> dict:
        """{이름: participant_id}. 옛 이름도 포함한다 (이름이 새 메시지에만 바뀌는 경우에도 같은 사람으로 알아본다).
        두 참가자가 같은 이름을 가졌던 적이 있으면 그 이름은 애매하므로 뺀다."""
        idx, dup = {}, set()
        for pid, p in self.participants.items():
            for n in p["names"]:
                if idx.get(n, pid) != pid:
                    dup.add(n)
                idx[n] = pid
        for n in dup:
            idx.pop(n, None)
        return idx

    def apply(self, ev: dict) -> None:
        t = ev["type"]
        if t in ("message.observed", "deleted_marker.observed", "system.observed"):
            self.registry[ev["message_id"]] = {
                "id": ev["message_id"], "kind": ev["kind"], "date": ev["date"],
                "hhmm": ev.get("hhmm"), "sender": ev.get("sender"), "text": ev["text"],
                "content_type": ev.get("content_type", "text"),
                "status": "active", "version": 1, "history": [],
                "attachment_id": None, "first_observed_at": ev["observed_at"],
                "participant_id": ev.get("participant_id"),
            }
        elif t == "message.deleted_for_everyone":
            self.registry[ev["message_id"]]["status"] = "deleted_for_everyone"
        elif t == "message.missing":
            self.registry[ev["message_id"]]["status"] = (
                "missing" if ev["range_covered"] else "unverifiable"
            )
        elif t == "message.reappeared":
            self.registry[ev["message_id"]]["status"] = "active"
        elif t == "message.edit_candidate":
            r = self.registry[ev["message_id"]]
            r["history"].append({"text": r["text"], "version": r["version"]})
            r["text"] = ev["current_text"]
            r["version"] += 1
        elif t in ("conversation.title_observed", "conversation.renamed"):
            new = ev["title"] if t == "conversation.title_observed" else ev["to"]
            self.titles.append({"title": new, "observed_at": ev["observed_at"]})
            self.title = new
        elif t == "participant.observed":
            self.participants[ev["participant_id"]] = {
                "id": ev["participant_id"], "names": [ev["name"]], "current": ev["name"]}
        elif t == "participant.renamed":
            p = self.participants[ev["participant_id"]]
            if ev["to"] not in p["names"]:
                p["names"].append(ev["to"])
            p["current"] = ev["to"]
            # 이름 변경은 내보내기에서 소급되어 시스템 줄 본문에도 반영된다 (실측). 기록도 같이 맞춘다.
            for r in self.registry.values():
                if r["kind"] == "system":
                    new_text = relabel_text(r["text"], ev["from"], ev["to"])
                    if new_text != r["text"]:
                        r["history"].append({"text": r["text"], "version": r["version"]})
                        r["text"], r["version"] = new_text, r["version"] + 1
        elif t == "participant.linked":
            keep, gone = self.participants[ev["participant_id"]], self.participants.pop(ev["merged_participant_id"])
            for n in gone["names"]:
                if n not in keep["names"]:
                    keep["names"].append(n)
            keep["current"] = ev["current_name"]
            for r in self.registry.values():
                if r.get("participant_id") == ev["merged_participant_id"]:
                    r["participant_id"] = ev["participant_id"]
        elif t == "attachment.saved":
            self.attachments[ev["attachment_id"]] = {
                k: ev[k] for k in ("attachment_id", "sha256", "filename", "size",
                                   "mime_type", "storage_key", "taken_at", "aliases")
            }
        elif t == "attachment.linked":
            self.registry[ev["message_id"]]["attachment_id"] = ev["attachment_id"]
        elif t == "export.ingested":
            self.visible = list(ev["visible"])
            self.ingests.append({k: ev[k] for k in ("export_name", "export_sha256", "saved_at")})
        elif t in ("attach.committed", "state.committed"):
            pass
        else:
            raise ValueError(f"알 수 없는 이벤트 타입: {t}")
        self.seq = max(self.seq, int(ev["event_id"].split("_")[1]))

    def known_export(self, sha256: str) -> bool:
        return any(i["export_sha256"] == sha256 for i in self.ingests)


def load(events_path: Path) -> tuple[State, int]:
    """(상태, 유효한 줄 수). 종결 이벤트 뒤의 꼬리 줄은 무시한다."""
    st = State()
    if not events_path.exists():
        return st, 0
    pending: list = []
    valid = 0
    for n, line in enumerate(events_path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        pending.append(json.loads(line))
        ver = pending[-1].get("schema_version", 1)
        if ver < SCHEMA_VERSION:
            raise SchemaError(
                f"{events_path} 는 스키마 v{ver} 로그다 (현재 v{SCHEMA_VERSION}). "
                "로그는 raw 내보내기에서 다시 만들 수 있다: 이 폴더를 지우고 ingest 를 다시 실행하라")
        if pending[-1]["type"] in TERMINATORS:
            for ev in pending:
                st.apply(ev)
            pending.clear()
            valid = n
    return st, valid


def append(events_path: Path, valid_lines: int, events: list) -> None:
    """트랜잭션을 추가한다. 이전의 불완전한 꼬리가 있으면 먼저 잘라낸다."""
    events_path.parent.mkdir(parents=True, exist_ok=True)
    if events_path.exists():
        lines = events_path.read_text(encoding="utf-8").splitlines()
        if len(lines) > valid_lines:
            events_path.write_text(
                "".join(l + "\n" for l in lines[:valid_lines]), encoding="utf-8"
            )
    with events_path.open("a", encoding="utf-8", newline="\n") as f:
        for ev in events:
            f.write(json.dumps(ev, ensure_ascii=False, sort_keys=True) + "\n")
        f.flush()
        os.fsync(f.fileno())
