from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

from .archive import Archive, decode_export
from .attach import ingest_attachments
from .parse import parse_export
from .link import resolve
from .reconcile import IngestError
from .state import SchemaError


def _arch(args) -> Archive:
    if not args.conversation:
        raise IngestError("--conversation 이 필요하다", "usage")
    return Archive(Path(args.archive), args.conversation)


def _parse_accept(values) -> dict:
    out = {}
    for v in values or []:
        old, sep, new = v.partition("=")
        if not sep or not old or not new:
            raise IngestError(f"--accept-rename 형식은 '옛이름=새이름' 이다: {v!r}", "usage")
        out[old] = new
    return out


def cmd_ingest(args) -> int:
    items = []
    for f in args.files:
        p = Path(f)
        parsed = parse_export(decode_export(p.read_bytes()))
        items.append((parsed.saved_at or "", p, parsed))
    items.sort(key=lambda t: (t[0], t[1].name))      # 시간순으로 반영해야 삭제/수정 판정이 맞다
    if not items:
        print("반영할 파일이 없다", file=sys.stderr)
        return 2
    try:
        accept = _parse_accept(args.accept_rename)
        for _, p, parsed in items:
            # 파일마다 방을 판정한다. 중간에 방 이름이 바뀐 내보내기가 섞여 있어도 이어 붙는다.
            conv, link = resolve(Path(args.archive), parsed, hint=args.conversation, force=args.force)
            r = Archive(Path(args.archive), conv).ingest(
                p, parsed=parsed, force=args.force, accept_renames=accept, link=link)
            print(json.dumps({"conversation": conv, "link": link["status"], **r}, ensure_ascii=False))
    except (IngestError, SchemaError) as e:
        print(f"[중단:{e.code}] {e}", file=sys.stderr)
        return 1
    return 0


def cmd_participants(args) -> int:
    state, _ = _arch(args).load()
    out = {"title": state.title, "titles": [t["title"] for t in state.titles],
           "participants": [{"id": p["id"], "current": p["current"], "names": p["names"],
                             "messages": sum(1 for r in state.registry.values()
                                             if r.get("participant_id") == p["id"])}
                            for p in state.participants.values()]}
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return 0


def cmd_participant_link(args) -> int:
    try:
        print(json.dumps(_arch(args).link_participants(args.keep, args.merge), ensure_ascii=False))
    except IngestError as e:
        print(f"[중단:{e.code}] {e}", file=sys.stderr)
        return 1
    return 0


def cmd_attach(args) -> int:
    arch = _arch(args)
    print(json.dumps(ingest_attachments(arch, Path(args.src), videos=not args.no_videos), ensure_ascii=False))
    return 0


def cmd_status(args) -> int:
    arch = _arch(args)
    state, _ = arch.load()
    msgs = [r for r in state.registry.values() if r["kind"] == "message"]
    out = {
        "conversation_id": arch.conversation_id,
        "title": state.title,
        "participants": len(state.participants),
        "exports_ingested": len(state.ingests),
        "last_export_saved_at": state.ingests[-1]["saved_at"] if state.ingests else None,
        "messages_by_status": dict(Counter(r["status"] for r in msgs)),
        "image_messages": sum(r["content_type"] == "image" for r in msgs),
        "image_messages_linked": sum(1 for r in msgs if r["content_type"] == "image" and len(r["attachment_ids"]) >= r["image_count"]),
        "deleted_markers_unmatched": sum(r["kind"] == "deleted_marker" for r in state.registry.values()),
        "attachments": len(state.attachments),
    }
    videos = [r for r in msgs if r["content_type"] == "video"]
    if videos:                                  # 동영상 메시지가 있을 때만 나타나는 키 (기존 출력은 그대로)
        out["video_messages"] = len(videos)
        out["video_messages_linked"] = sum(1 for r in videos if r["attachment_ids"])
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="kkt", description="카카오톡 내보내기 아카이버")
    ap.add_argument("--archive", default="data/archive", help="아카이브 루트 (기본 data/archive)")
    ap.add_argument("--conversation", help="대화방 ID. ingest 에서는 생략하면 제목과 메시지 겹침으로 판정")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("ingest", help="내보내기 TXT를 반영")
    p.add_argument("files", nargs="+")
    p.add_argument("--force", action="store_true", help="대량 소실·방 불일치 안전장치를 무시")
    p.add_argument("--accept-rename", action="append", metavar="OLD=NEW",
                   help="멤버 이름 변경을 사람이 확인해 확정 (근거가 약할 때). 여러 번 지정 가능")
    p.set_defaults(fn=cmd_ingest)
    p = sub.add_parser("attach", help="저장한 사진 폴더를 반영하고 메시지와 연결")
    p.add_argument("src")
    p.add_argument("--no-videos", action="store_true", help="동영상(.mp4)은 보관하지도 연결하지도 않는다")
    p.set_defaults(fn=cmd_attach)
    p = sub.add_parser("status", help="요약 출력")
    p.set_defaults(fn=cmd_status)
    p = sub.add_parser("participants", help="참가자와 이름 이력 출력")
    p.set_defaults(fn=cmd_participants)
    p = sub.add_parser("participant-link", help="두 참가자를 같은 사람으로 확정 (수동)")
    p.add_argument("--keep", required=True, help="유지할 참가자의 현재 이름 (옛 이름)")
    p.add_argument("--merge", required=True, help="합칠 참가자의 현재 이름 (새 이름)")
    p.set_defaults(fn=cmd_participant_link)
    args = ap.parse_args(argv)
    try:
        return args.fn(args)
    except (IngestError, SchemaError) as e:
        print(f"[중단:{e.code}] {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
