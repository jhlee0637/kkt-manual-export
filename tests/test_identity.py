import json

import pytest

from kkt.archive import Archive
from kkt.link import resolve
from kkt.parse import parse_export
from kkt.reconcile import IngestError
from helpers import make_export, write_export

CHAT = [
    "[alice] [오전 9:00] hello",
    "[bob] [오전 9:01] hi",
    "[alice] [오전 9:02] how are you",
    "[bob] [오전 9:03] fine",
    "[alice] [오전 9:04] great",
]


def sub(lines, old, new):
    return [l.replace(f"[{old}]", f"[{new}]") for l in lines]


def events(arch, *types):
    out = [json.loads(l) for l in arch.events_path.read_text(encoding="utf-8").splitlines()]
    return [e for e in out if not types or e["type"] in types]


def names(arch):
    st, _ = arch.load()
    return {p["current"] for p in st.participants.values()}


def ingest_resolved(root, tmp, name, saved, lines, title="room", **kw):
    """CLI 와 같은 경로: 방을 판정한 뒤 반영한다."""
    path = write_export(tmp, name, saved, lines, title=title)
    parsed = parse_export(path.read_text(encoding="utf-8"))
    conv, link = resolve(root, parsed, hint=kw.pop("hint", None), force=kw.get("force", False))
    r = Archive(root, conv).ingest(path, parsed=parsed, link=link, **kw)
    return conv, link, r


# ── 방 이름 ──────────────────────────────────────────────
def test_room_rename_is_linked_by_message_overlap(tmp_path):
    root = tmp_path / "a"
    c1, l1, _ = ingest_resolved(root, tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT, title="old room")
    assert l1["status"] == "new"
    c2, l2, r = ingest_resolved(root, tmp_path, "e2.txt", "2026-10-02 10:10:00",
                                CHAT + ["[bob] [오전 9:10] new"], title="new room")
    assert c2 == c1 and l2["status"] == "renamed_room" and l2["basis"] == "message_overlap"
    arch = Archive(root, c1)
    ren = events(arch, "conversation.renamed")
    assert len(ren) == 1 and ren[0]["from"] == "old room" and ren[0]["to"] == "new room"
    assert ren[0]["link"]["matched"] == 5
    st, _ = arch.load()
    assert st.title == "new room" and [t["title"] for t in st.titles] == ["old room", "new room"]
    assert r["events"] == {"message.observed": 1, "conversation.renamed": 1}      # 기존 메시지는 그대로


def test_same_title_links_without_rename_event(tmp_path):
    root = tmp_path / "a"
    c1, _, _ = ingest_resolved(root, tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT)
    c2, l2, _ = ingest_resolved(root, tmp_path, "e2.txt", "2026-10-02 10:10:00", CHAT + ["[bob] [오전 9:10] x"])
    assert c2 == c1 and l2["status"] == "same"
    assert not events(Archive(root, c1), "conversation.renamed")


def test_unrelated_new_title_becomes_new_conversation(tmp_path):
    root = tmp_path / "a"
    c1, _, _ = ingest_resolved(root, tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT, title="A")
    other = ["[carol] [오전 8:00] totally", "[dave] [오전 8:01] different", "[carol] [오전 8:02] chat"]
    c2, l2, _ = ingest_resolved(root, tmp_path, "e2.txt", "2026-10-02 10:10:00", other, title="B")
    assert c2 != c1 and l2["status"] == "new"


def test_same_title_but_different_content_is_ambiguous(tmp_path):
    """같은 이름의 다른 방(또는 내용이 통째로 바뀐 방)을 말없이 합치지 않는다."""
    root = tmp_path / "a"
    ingest_resolved(root, tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT, title="room")
    other = ["[carol] [오전 8:00] totally", "[dave] [오전 8:01] different", "[carol] [오전 8:02] chat"]
    with pytest.raises(IngestError):
        ingest_resolved(root, tmp_path, "e2.txt", "2026-10-02 10:10:00", other, title="room")


def test_partial_overlap_with_new_title_is_ambiguous(tmp_path):
    root = tmp_path / "a"
    ingest_resolved(root, tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT, title="A")
    half = CHAT[:2] + ["[carol] [오전 9:30] x", "[carol] [오전 9:31] y", "[carol] [오전 9:32] z"]
    with pytest.raises(IngestError):
        ingest_resolved(root, tmp_path, "e2.txt", "2026-10-02 10:10:00", half, title="B")


def test_explicit_conversation_with_no_overlap_needs_force(tmp_path):
    root = tmp_path / "a"
    c1, _, _ = ingest_resolved(root, tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT)
    other = ["[carol] [오전 8:00] a", "[dave] [오전 8:01] b", "[carol] [오전 8:02] c"]
    with pytest.raises(IngestError):
        ingest_resolved(root, tmp_path, "e2.txt", "2026-10-02 10:10:00", other, title="x", hint=c1)


# ── 멤버 이름 ────────────────────────────────────────────
def test_member_rename_retroactive_is_confirmed_and_persistent(tmp_path):
    arch = Archive(tmp_path / "a", "kt")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT))
    renamed = sub(CHAT, "alice", "alice2") + ["[alice2] [오전 9:10] new msg"]
    r = arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", renamed))
    # 이전 메시지가 missing 이 되거나 새로 관측되지 않는다. 이름 변경 1건과 새 메시지 1건뿐이다.
    assert r["events"] == {"participant.renamed": 1, "message.observed": 1}
    ev = events(arch, "participant.renamed")[0]
    assert (ev["from"], ev["to"], ev["matched_messages"], ev["basis"]) == ("alice", "alice2", 3, "consistent_relabel")
    assert names(arch) == {"alice2", "bob"}
    st, _ = arch.load()
    assert len(st.participants) == 2                       # 같은 사람으로 이어짐
    # 이후 내보내기(여전히 새 이름)도 이벤트 없이 이어진다
    r = arch.ingest(write_export(tmp_path, "e3.txt", "2026-10-02 10:20:00", renamed + ["[bob] [오전 9:11] ok"]))
    assert r["events"] == {"message.observed": 1}


def test_single_message_rename_is_not_auto_confirmed(tmp_path):
    arch = Archive(tmp_path / "a", "kt")
    chat = ["[alice] [오전 9:00] one", "[bob] [오전 9:01] two", "[bob] [오전 9:02] three"]
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", chat))
    changed = sub(chat, "alice", "alice2")
    r = arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", changed), force=True)
    assert "participant.renamed" not in r["events"]
    assert any("alice" in w and "--accept-rename" in w for w in r["warnings"])


def test_accept_rename_confirms_weak_evidence(tmp_path):
    arch = Archive(tmp_path / "a", "kt")
    chat = ["[alice] [오전 9:00] one", "[bob] [오전 9:01] two", "[bob] [오전 9:02] three"]
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", chat))
    r = arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", sub(chat, "alice", "alice2")),
                    accept_renames={"alice": "alice2"})
    assert r["events"] == {"participant.renamed": 1}
    assert events(arch, "participant.renamed")[0]["basis"] == "forced_by_user"


def test_partial_relabel_is_not_a_rename(tmp_path):
    """옛 이름이 새 내보내기에도 남아 있으면 이름 변경이 아니다 (다른 사람일 수 있다)."""
    arch = Archive(tmp_path / "a", "kt")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT))
    mixed = list(CHAT)
    mixed[0] = mixed[0].replace("alice", "alice2")
    mixed[2] = mixed[2].replace("alice", "alice2")          # alice 의 메시지 3개 중 2개만 바뀜
    r = arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", mixed), force=True)
    assert "participant.renamed" not in r["events"]


def test_rename_to_an_existing_participant_is_not_confirmed(tmp_path):
    """새 이름이 이미 이 방의 다른 참가자라면 두 사람을 합치는 것이므로 자동 확정하지 않는다."""
    arch = Archive(tmp_path / "a", "kt")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT))
    r = arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", sub(CHAT, "alice", "bob")), force=True)
    assert "participant.renamed" not in r["events"]


def test_non_retroactive_name_is_a_new_participant_until_manually_linked(tmp_path):
    """새 이름이 새 메시지에만 나타나면(과거 메시지는 옛 이름 유지) TXT 에 근거가 없다. 수동 연결로 이어 붙인다."""
    arch = Archive(tmp_path / "a", "kt")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT))
    r = arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", CHAT + ["[alice2] [오전 9:10] i changed"]))
    assert r["events"] == {"participant.observed": 1, "message.observed": 1}
    assert names(arch) == {"alice", "alice2", "bob"}
    res = arch.link_participants(keep_name="alice", merge_name="alice2")
    assert res["current_name"] == "alice2"
    st, _ = arch.load()
    assert len(st.participants) == 2 and names(arch) == {"alice2", "bob"}
    alice = next(p for p in st.participants.values() if p["current"] == "alice2")
    assert alice["names"] == ["alice", "alice2"]
    # 합친 뒤에도 같은 내보내기를 다시 넣거나 이어서 넣어도 이벤트가 새지 않는다
    r = arch.ingest(write_export(tmp_path, "e3.txt", "2026-10-02 10:20:00", CHAT + ["[alice2] [오전 9:10] i changed", "[bob] [오전 9:12] z"]))
    assert r["events"] == {"message.observed": 1}


def test_link_participants_validates_names(tmp_path):
    arch = Archive(tmp_path / "a", "kt")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT))
    with pytest.raises(IngestError):
        arch.link_participants("alice", "nobody")
    with pytest.raises(IngestError):
        arch.link_participants("alice", "alice")


def test_old_schema_log_is_rejected_not_misread(tmp_path):
    from kkt.state import SchemaError
    arch = Archive(tmp_path / "a", "kt")
    arch.events_path.parent.mkdir(parents=True)
    arch.events_path.write_text(
        json.dumps({"type": "export.ingested", "event_id": "ev_000001", "schema_version": 1,
                    "visible": [], "export_name": "x", "export_sha256": "x", "saved_at": "x"}) + "\n",
        encoding="utf-8")
    with pytest.raises(SchemaError):
        arch.load()


def test_rename_is_confirmed_by_message_plus_system_line_evidence(tmp_path):
    """실측: 이름 변경은 과거 메시지뿐 아니라 초대 시스템 줄 본문에도 소급된다.
    메시지 근거가 1건뿐이어도 시스템 줄의 같은 치환이 두 번째 근거가 된다."""
    arch = Archive(tmp_path / "a", "kt")
    chat = [".님이 alice님, bob님을 초대했습니다.", "[me] [오전 9:00] hi", "[alice] [오전 9:01] one",
            "[bob] [오전 9:02] two", "[bob] [오전 9:03] three"]
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", chat))
    changed = [".님이 alice2님, bob님을 초대했습니다."] + sub(chat[1:], "alice", "alice2")
    r = arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", changed))
    assert r["events"] == {"participant.renamed": 1}          # 시스템 줄은 새로 관측되지도 missing 도 되지 않는다
    ev = events(arch, "participant.renamed")[0]
    assert ev["evidence"] == {"messages": 1, "system_lines": 1} and ev["matched_messages"] == 2
    # 상태의 시스템 줄 본문도 새 이름으로 맞춰지고, 다음 내보내기도 이어진다
    r = arch.ingest(write_export(tmp_path, "e3.txt", "2026-10-02 10:20:00", changed + ["[bob] [오전 9:10] x"]))
    assert r["events"] == {"message.observed": 1}
    st, _ = arch.load()
    sysrec = next(x for x in st.registry.values() if x["kind"] == "system")
    assert sysrec["text"] == ".님이 alice2님, bob님을 초대했습니다." and sysrec["history"][0]["text"].startswith(".님이 alice님")


def test_period_name_does_not_corrupt_system_text(tmp_path):
    """이름이 '.' 인 참가자(나)가 바뀌어도 문장 끝의 '.' 은 건드리지 않는다."""
    from kkt.state import relabel_text
    assert relabel_text(".님이 A님을 초대했습니다.", ".", "me") == "me님이 A님을 초대했습니다."


def test_rename_back_to_previous_name_is_confirmed(tmp_path):
    """실측: 이름을 되돌리면 내보내기가 과거 메시지와 시스템 줄에도 옛 이름을 되돌려 찍는다."""
    arch = Archive(tmp_path / "a", "kt")
    chat = [".님이 alice님, bob님을 초대했습니다.", "[me] [오전 9:00] hi", "[alice] [오전 9:01] one",
            "[bob] [오전 9:02] two", "[bob] [오전 9:03] three"]
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", chat))
    changed = [".님이 alice2님, bob님을 초대했습니다."] + sub(chat[1:], "alice", "alice2")
    arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", changed))
    r = arch.ingest(write_export(tmp_path, "e3.txt", "2026-10-02 10:20:00", chat))      # 되돌림
    assert r["events"] == {"participant.renamed": 1}
    ev = events(arch, "participant.renamed")[-1]
    assert (ev["from"], ev["to"], ev["basis"]) == ("alice2", "alice", "restored_previous_name")
    st, _ = arch.load()
    p = next(x for x in st.participants.values() if x["current"] == "alice")
    assert p["names"] == ["alice", "alice2"] and len(st.participants) == 3        # 사람은 그대로 3명
    sysrec = next(x for x in st.registry.values() if x["kind"] == "system")
    assert sysrec["text"] == ".님이 alice님, bob님을 초대했습니다."
    assert arch.ingest(write_export(tmp_path, "e4.txt", "2026-10-02 10:30:00", chat + ["[bob] [오전 9:10] x"]))["events"] == {"message.observed": 1}


def test_non_retroactive_world_stays_quiet_after_manual_link(tmp_path):
    """수동 연결 뒤 옛 메시지는 계속 옛 이름으로 찍힌다. 이것은 정상이므로 경고도 이벤트도 없어야 한다."""
    arch = Archive(tmp_path / "a", "kt")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 10:00:00", CHAT))
    arch.ingest(write_export(tmp_path, "e2.txt", "2026-10-02 10:10:00", CHAT + ["[alice2] [오전 9:10] new"]))
    arch.link_participants("alice", "alice2")
    r = arch.ingest(write_export(tmp_path, "e3.txt", "2026-10-02 10:20:00", CHAT + ["[alice2] [오전 9:10] new", "[bob] [오전 9:11] z"]))
    assert r["events"] == {"message.observed": 1} and r["warnings"] == []
