from kkt.parse import parse_export, to_24h
from helpers import make_export


def test_to_24h_edges():
    assert to_24h("오전", 12) == 0       # 오전 12시 = 자정
    assert to_24h("오후", 12) == 12      # 오후 12시 = 정오
    assert to_24h("오전", 9) == 9
    assert to_24h("오후", 1) == 13


def test_header_and_kinds():
    txt = make_export("2026-10-02 10:18:07", [
        ".님이 A님, B님을 초대했습니다.",
        "[me] [오전 9:27] hello",
        "[me] [오전 9:28] 사진",
        "메시지가 삭제되었습니다.",
        "[bob] [오후 12:05] noon",
        "[bob] [오전 12:05] midnight",
    ])
    p = parse_export(txt)
    assert p.title == "room"
    assert p.saved_at == "2026-10-02T10:18:07+09:00"
    assert [e.kind for e in p.entries] == ["system", "message", "message", "deleted_marker", "message", "message"]
    assert p.entries[1].hhmm == "09:27" and p.entries[1].sender == "me"
    assert p.entries[2].content_type == "image"
    assert p.entries[3].hhmm is None and p.entries[3].sender is None
    assert p.entries[4].hhmm == "12:05" and p.entries[5].hhmm == "00:05"
    assert p.first_date == p.last_date == "2026-10-02"
    assert p.warnings == []


def test_multiline_message_and_trailing_blank_lines():
    txt = make_export("2026-10-02 10:00:00", [
        "[me] [오전 9:00] line1",
        "line2",
        "",
        "line4",
        "[me] [오전 9:01] next",
    ])
    p = parse_export(txt)
    assert p.entries[0].text == "line1\nline2\n\nline4"
    assert p.entries[1].text == "next"


def test_bom_and_crlf():
    txt = "﻿" + make_export("2026-10-02 10:00:00", ["[me] [오전 9:00] x"])
    p = parse_export(txt)
    assert p.title == "room" and len(p.entries) == 1


def test_multiple_date_headers():
    txt = make_export("2026-10-03 08:00:00", ["[me] [오후 11:59] late"]) + \
          "--------------- 2026년 10월 3일 토요일 ---------------\r\n[me] [오전 12:01] early\r\n"
    p = parse_export(txt)
    assert [(e.date, e.hhmm) for e in p.entries] == [("2026-10-02", "23:59"), ("2026-10-03", "00:01")]
    assert p.first_date == "2026-10-02" and p.last_date == "2026-10-03"


def test_text_that_looks_like_image_but_is_not():
    p = parse_export(make_export("2026-10-02 10:00:00", ["[me] [오전 9:00] 사진 찍었어"]))
    assert p.entries[0].content_type == "text"


def test_multi_photo_line_has_count():
    """실측: 한 번에 여러 장을 보내면 '사진 16장' 한 줄이다."""
    p = parse_export(make_export("2026-10-06 08:11:32", [
        "[.] [오전 8:07] 사진 16장", "[.] [오전 8:07] 사진 2장", "[.] [오전 8:07] 사진", "[.] [오전 8:08] 사진 1장"]))
    assert [(e.content_type, e.image_count) for e in p.entries] == [("image", 16), ("image", 2), ("image", 1), ("image", 1)]


def test_photo_like_text_is_not_image():
    p = parse_export(make_export("2026-10-06 08:11:32", [
        "[.] [오전 8:07] 사진 0장", "[.] [오전 8:07] 사진 2장 찍었어", "[.] [오전 8:07] 사진 장", "[.] [오전 8:07] 사진  2장"]))
    assert [e.content_type for e in p.entries] == ["text"] * 4


def test_emoticon_line_and_reply_looks_like_plain_message():
    """실측: 이모티콘은 '이모티콘' 한 줄, 답장은 인용 없이 본문만 일반 메시지로 나온다."""
    p = parse_export(make_export("2026-10-02 10:40:18", [
        "[me] [오전 10:39] reply",
        "[me] [오전 10:39] reply",
        "[me] [오전 10:39] 이모티콘",
    ]))
    assert [e.content_type for e in p.entries] == ["text", "text", "emoticon"]
    a, b = p.entries[0], p.entries[1]
    assert (a.date, a.hhmm, a.sender, a.text) == (b.date, b.hhmm, b.sender, b.text)   # 같은 분, 같은 내용: 구분은 순서로만 가능
