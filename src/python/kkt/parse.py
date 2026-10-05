"""카카오톡 PC '대화 내보내기' TXT 파서.

관측된 형식 (2026-10-02, test 방):

    test 님과 카카오톡 대화
    저장한 날짜 : 2026-10-02 10:18:07

    --------------- 2026년 10월 2일 금요일 ---------------
    .님이 A님, B님을 초대했습니다.            <- 시스템 이벤트
    [.] [오전 9:27] Test                      <- 메시지
    [.] [오전 9:28] 사진                      <- 사진은 '사진' 한 줄 (한 번에 여러 장이면 '사진 16장' 한 줄)
    메시지가 삭제되었습니다.                  <- '모두에게 삭제'. 보낸이/시각 없음

관측으로 알게 된 한계:
- 시각은 분 단위다. 초는 없다.
- '나에게서만 삭제'한 메시지는 줄이 사라지고 흔적이 없다.
- 수정된 메시지에는 표시가 없다.
- 답장(인용)은 TXT에 **아무 흔적도 없다**. 화면에는 '○○에게 답장 / 인용문 / 본문'으로 보이지만
  내보내기에는 본문만 일반 메시지로 나온다. 답장 관계는 TXT로는 복원할 수 없다.
- 사진은 '사진', 이모티콘은 '이모티콘' 한 줄이다. 한 번에 여러 장 보내면 '사진 N장' 한 줄이다 (실측).
- 동영상, 파일, 링크의 형식은 아직 관측하지 못했다. 알 수 없는 형식은 추측하지 않고 text로 둔다.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Optional

DELETED_MARKER = "메시지가 삭제되었습니다."

_TITLE_RE = re.compile(r"^(?P<title>.*) 님과 카카오톡 대화$")
_SAVED_RE = re.compile(
    r"^저장한 날짜 : (?P<y>\d{4})-(?P<mo>\d{2})-(?P<d>\d{2}) (?P<h>\d{2}):(?P<mi>\d{2}):(?P<s>\d{2})$"
)
_DATE_RE = re.compile(
    r"^-{3,} (?P<y>\d{4})년 (?P<mo>\d{1,2})월 (?P<d>\d{1,2})일 \S*요일 -{3,}$"
)
_MSG_RE = re.compile(
    r"^\[(?P<sender>.+?)\] \[(?P<ampm>오전|오후) (?P<h>\d{1,2}):(?P<mi>\d{2})\] (?P<text>.*)$"
)
# 관측된 시스템 문구만 등록한다. 여기 없는 줄은 앞 메시지의 이어지는 줄로 본다.
_SYSTEM_RES = [
    re.compile(r".+님이 .+님을 초대했습니다\.$"),
    re.compile(r".+님이 들어왔습니다\.$"),
    re.compile(r".+님이 나갔습니다\.$"),
]
# 사진 한 장은 "사진", 여러 장을 한 번에 보내면 "사진 16장" 한 줄이다 (실측). 글자 그대로 "사진 2장"이라고 보낸 메시지와는 TXT만으로 구분할 수 없다.
_IMAGE_RE = re.compile(r"^사진(?: ([0-9]+)장)?$")
# 이모티콘도 '이모티콘' 한 줄로만 나온다 (어떤 이모티콘인지는 알 수 없다).
# 사용자가 글자 그대로 '이모티콘'이라고 보낸 메시지와는 TXT만으로 구분할 수 없다.
_EMOTICON_RE = re.compile(r"^이모티콘$")


@dataclass
class Entry:
    """내보내기 한 줄(또는 여러 줄 메시지 하나)."""

    kind: str                       # message | deleted_marker | system
    line_no: int                    # 1부터, 시작 줄
    date: Optional[str] = None      # YYYY-MM-DD (날짜 헤더 기준)
    hhmm: Optional[str] = None      # HH:MM (24시간). deleted_marker/system은 None
    sender: Optional[str] = None
    text: str = ""
    content_type: str = "text"      # text | image
    image_count: int = 1            # content_type 이 image 일 때 사진 수 ('사진 16장' 이면 16)
    raw_lines: list = field(default_factory=list)


@dataclass
class ParsedExport:
    title: Optional[str]
    saved_at: Optional[str]         # 'YYYY-MM-DDTHH:MM:SS+09:00'
    entries: list
    first_date: Optional[str]
    last_date: Optional[str]
    warnings: list = field(default_factory=list)


def to_24h(ampm: str, hour: int) -> int:
    if ampm == "오전":
        return 0 if hour == 12 else hour
    return 12 if hour == 12 else hour + 12


def split_lines(text: str) -> list:
    r"""줄 구분은 \r\n, \n, \r 만이다. str.splitlines() 는 U+2028, U+0085, \x0b 등으로도 나눠서
    메시지 본문을 조용히 바꾸므로 쓰지 않는다 (다른 언어로 이식할 때도 이 정의를 따른다)."""
    parts = re.split(r"\r\n|\n|\r", text)
    if parts and parts[-1] == "":
        parts.pop()
    return parts


def _is_system(line: str) -> bool:
    return any(r.match(line) for r in _SYSTEM_RES)


def parse_export(text: str) -> ParsedExport:
    lines = split_lines(text.lstrip("\ufeff"))
    title = saved_at = None
    entries: list = []
    warnings: list = []
    cur_date: Optional[str] = None
    cur: Optional[Entry] = None

    def close_current():
        nonlocal cur
        if cur is not None:
            while cur.raw_lines and cur.raw_lines[-1] == "":
                cur.raw_lines.pop()
            cur.text = "\n".join(cur.raw_lines)
            m_img = _IMAGE_RE.match(cur.text) if cur.kind == "message" else None
            if m_img and (m_img.group(1) is None or int(m_img.group(1)) >= 1):
                cur.content_type = "image"
                cur.image_count = int(m_img.group(1)) if m_img.group(1) else 1
            elif cur.kind == "message" and _EMOTICON_RE.match(cur.text):
                cur.content_type = "emoticon"
            entries.append(cur)
            cur = None

    for i, raw in enumerate(lines, start=1):
        line = raw.rstrip("\r")

        if i <= 3:
            m = _TITLE_RE.match(line)
            if m and title is None:
                title = m.group("title")
                continue
            m = _SAVED_RE.match(line)
            if m and saved_at is None:
                saved_at = (
                    f"{m['y']}-{m['mo']}-{m['d']}T{m['h']}:{m['mi']}:{m['s']}+09:00"
                )
                continue

        m = _DATE_RE.match(line)
        if m:
            close_current()
            cur_date = f"{int(m['y']):04d}-{int(m['mo']):02d}-{int(m['d']):02d}"
            continue

        if cur_date is None:
            # 첫 날짜 헤더 전의 줄(헤더, 빈 줄)은 무시
            continue

        m = _MSG_RE.match(line)
        if m:
            close_current()
            hh = to_24h(m["ampm"], int(m["h"]))
            cur = Entry(
                kind="message", line_no=i, date=cur_date,
                hhmm=f"{hh:02d}:{m['mi']}", sender=m["sender"],
                raw_lines=[m["text"]],
            )
            continue

        if line == DELETED_MARKER:
            close_current()
            entries.append(Entry(kind="deleted_marker", line_no=i, date=cur_date,
                                 text=line, raw_lines=[line]))
            continue

        if _is_system(line):
            close_current()
            entries.append(Entry(kind="system", line_no=i, date=cur_date,
                                 text=line, raw_lines=[line]))
            continue

        # 그 외: 앞 메시지의 이어지는 줄 (빈 줄 포함)
        if cur is not None:
            cur.raw_lines.append(line)
        elif line.strip():
            warnings.append(f"line {i}: 해석할 수 없는 줄 (무시): {line[:40]!r}")

    close_current()

    dates = [e.date for e in entries if e.date]
    if title is None:
        warnings.append("제목 줄을 찾지 못했다")
    if saved_at is None:
        warnings.append("'저장한 날짜' 줄을 찾지 못했다")
    return ParsedExport(
        title=title, saved_at=saved_at, entries=entries,
        first_date=min(dates) if dates else None,
        last_date=max(dates) if dates else None,
        warnings=warnings,
    )
