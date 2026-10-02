import json

import pytest

from kkt.archive import Archive
from kkt.reconcile import IngestError
from helpers import write_export

BASE = [
    ".님이 A님을 초대했습니다.",
    "[me] [오전 9:27] Test",
    "[me] [오전 9:27] Test",          # 같은 분, 같은 내용이 연속
    "[me] [오전 9:28] 사진",
    "[bob] [오전 9:33] hi",
    "[bob] [오전 9:42] yo",
    "[me] [오전 10:06] t1",
    "[me] [오전 10:06] t2",
]


def events_of(arch, *types):
    out = [json.loads(l) for l in arch.events_path.read_text(encoding="utf-8").splitlines()]
    return [e for e in out if not types or e["type"] in types]


def status_by_text(arch):
    state, _ = arch.load()
    return {(r["text"], r["status"]) for r in state.registry.values() if r["kind"] == "message"}


@pytest.fixture
def arch(tmp_path):
    return Archive(tmp_path / "a", "kt_test")


def ingest(arch, tmp_path, name, saved, lines, **kw):
    return arch.ingest(write_export(tmp_path, name, saved, lines), **kw)


def test_first_ingest_and_identical_messages_get_distinct_ids(arch, tmp_path):
    r = ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    assert r["events"]["message.observed"] == 7
    obs = events_of(arch, "message.observed")
    tests = [e for e in obs if e["text"] == "Test"]
    assert len(tests) == 2 and tests[0]["message_id"] != tests[1]["message_id"]


def test_reingest_same_file_is_noop(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    n = len(events_of(arch))
    r = ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    assert r["skipped"] and len(events_of(arch)) == n


def test_new_messages_only_add_events(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    r = ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", BASE + ["[bob] [오전 10:15] new"])
    assert r["events"] == {"message.observed": 1}


def test_delete_for_everyone_is_confirmed_in_place(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    lines = list(BASE)
    lines[3] = "메시지가 삭제되었습니다."          # 사진이 삭제 표식으로 치환
    r = ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", lines)
    assert r["events"] == {"message.deleted_for_everyone": 1}
    ev = events_of(arch, "message.deleted_for_everyone")[0]
    assert ev["inferred_after"] == "2026-10-02T09:27:00+09:00"
    assert ev["inferred_before"] == "2026-10-02T09:33:00+09:00"
    # 삭제 표식은 다음 내보내기에서도 같은 항목으로 유지된다 (새 이벤트 없음)
    r = ingest(arch, tmp_path, "e3.txt", "2026-10-02 10:30:00", lines)
    assert r["events"] == {}


def test_local_only_delete_is_missing_not_deleted(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    lines = [l for l in BASE if "yo" not in l]
    r = ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", lines)
    assert r["events"] == {"message.missing": 1}
    assert ("yo", "missing") in status_by_text(arch)
    assert not events_of(arch, "message.deleted_for_everyone")


def test_edit_is_a_candidate_with_history(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    lines = [l.replace("] t2", "] t2-2") for l in BASE]
    r = ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", lines)
    assert r["events"] == {"message.edit_candidate": 1}
    ev = events_of(arch, "message.edit_candidate")[0]
    assert (ev["previous_text"], ev["current_text"], ev["confidence"]) == ("t2", "t2-2", "candidate")
    state, _ = arch.load()
    rec = state.registry[ev["message_id"]]
    assert rec["text"] == "t2-2" and rec["version"] == 2 and rec["history"][0]["text"] == "t2"
    assert rec["status"] == "active"


def test_delete_edit_and_missing_together(arch, tmp_path):
    """실측 시나리오: 한 번에 삭제(모두), 수정, 그리고 다음 내보내기에서 로컬 삭제."""
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    l2 = list(BASE)
    l2[3] = "메시지가 삭제되었습니다."
    l2[7] = "[me] [오전 10:06] t2-2"
    r = ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", l2)
    assert r["events"] == {"message.deleted_for_everyone": 1, "message.edit_candidate": 1}
    l3 = [l for l in l2 if "yo" not in l and "t2-2" not in l]
    r = ingest(arch, tmp_path, "e3.txt", "2026-10-02 10:30:00", l3)
    assert r["events"] == {"message.missing": 2}


def test_ambiguous_same_minute_edit_is_not_paired(arch, tmp_path):
    """같은 분/보낸이 메시지가 여럿이고 개수가 다르면 수정으로 짝짓지 않는다."""
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    lines = [l for l in BASE if "t1" not in l]               # 10:06 두 개 중 하나가 사라지고
    lines = [l.replace("] t2", "] t2-2") for l in lines]      # 남은 하나는 내용이 바뀜
    r = ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", lines)
    assert "message.edit_candidate" not in r["events"]
    assert r["events"]["message.missing"] == 2 and r["events"]["message.observed"] == 1


def test_reappeared_message_reuses_id(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", [l for l in BASE if "yo" not in l])
    r = ingest(arch, tmp_path, "e3.txt", "2026-10-02 10:30:00", BASE)
    assert r["events"] == {"message.reappeared": 1}
    assert ("yo", "active") in status_by_text(arch)


def test_history_trimmed_is_unverifiable_not_missing(arch, tmp_path):
    """새 내보내기가 더 늦은 날짜부터 시작하면, 그 이전 항목이 없어도 '삭제'로 보지 않는다."""
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    only_later = write_export(tmp_path, "e2.txt", "2026-10-03 09:00:00",
                              ["[me] [오전 9:00] fresh"], date_hdr="2026년 10월 3일 토요일")
    r = arch.ingest(only_later, force=True)
    ev = events_of(arch, "message.missing")
    assert ev and all(e["range_covered"] is False for e in ev)
    state, _ = arch.load()
    assert {x["status"] for x in state.registry.values() if x["kind"] == "message"} <= {"unverifiable", "active"}


def test_mass_loss_requires_force(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    with pytest.raises(IngestError):
        ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", [BASE[1]])
    assert len(events_of(arch, "export.ingested")) == 1          # 아무것도 기록되지 않았다
    ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", [BASE[1]], force=True)
    assert len(events_of(arch, "export.ingested")) == 2


def test_empty_export_is_rejected(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    with pytest.raises(IngestError):
        ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", [])


def test_incomplete_tail_is_ignored_and_truncated(arch, tmp_path):
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:10:00", BASE)
    good = arch.events_path.read_text(encoding="utf-8")
    with arch.events_path.open("a", encoding="utf-8") as f:     # 종결 이벤트 없는 꼬리 (크래시 흉내)
        f.write(json.dumps({"type": "message.observed", "event_id": "ev_999999", "schema_version": 2}) + "\n")
    state, valid = arch.load()
    assert len(state.visible) == len(BASE) and state.seq < 999999
    r = ingest(arch, tmp_path, "e2.txt", "2026-10-02 10:20:00", BASE + ["[bob] [오전 10:15] n"])
    assert r["events"] == {"message.observed": 1}
    assert "ev_999999" not in arch.events_path.read_text(encoding="utf-8")
    assert arch.events_path.read_text(encoding="utf-8").startswith(good)


def test_emoticon_and_unknown_reply_are_recorded_without_guessing(arch, tmp_path):
    lines = BASE + ["[me] [오전 10:39] reply", "[me] [오전 10:39] reply", "[me] [오전 10:39] 이모티콘"]
    ingest(arch, tmp_path, "e1.txt", "2026-10-02 10:40:00", lines)
    obs = {e["message_id"]: e for e in events_of(arch, "message.observed")}
    replies = [e for e in obs.values() if e["text"] == "reply"]
    assert len(replies) == 2 and replies[0]["message_id"] != replies[1]["message_id"]
    assert all(e["reply_to"] == {"status": "unknown_from_txt"} for e in obs.values())
    emo = [e for e in obs.values() if e["content_type"] == "emoticon"]
    assert len(emo) == 1 and emo[0]["content"] == [{"type": "emoticon"}]
