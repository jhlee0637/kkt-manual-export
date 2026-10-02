"""테스트용 합성 내보내기 생성기. 실제 대화 내용은 저장소에 넣지 않는다."""
from pathlib import Path


def make_export(saved: str, lines: list, title: str = "room", date_hdr: str = "2026년 10월 2일 금요일") -> str:
    """saved: '2026-10-02 10:18:07'. lines: 날짜 헤더 아래에 그대로 들어갈 줄들."""
    out = [f"{title} 님과 카카오톡 대화", f"저장한 날짜 : {saved}", "",
           f"--------------- {date_hdr} ---------------", *lines]
    return "\r\n".join(out) + "\r\n"


def write_export(tmp: Path, name: str, saved: str, lines: list, **kw) -> Path:
    p = tmp / name
    p.write_bytes(make_export(saved, lines, **kw).encode("utf-8"))
    return p
