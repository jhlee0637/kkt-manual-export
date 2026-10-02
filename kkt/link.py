"""새 내보내기가 어느 대화방에 속하는지 판정한다.

방 제목은 바뀔 수 있고 TXT에는 방 ID가 없다. 그래서 제목 일치 여부와 함께
'이전에 본 메시지들이 새 내보내기에도 같은 순서로 있는가'(겹침 비율)를 증거로 쓴다.
발신자 이름도 바뀔 수 있으므로 겹침은 (날짜, 분, 내용)으로만 비교한다.

판정:
  same          제목이 같고 겹침이 충분하거나 비교할 근거가 없음
  renamed_room  제목은 없지만 한 방과 충분히, 다른 방과는 거의 겹치지 않음 -> 이름 변경으로 본다
  new           어느 방과도 겹치지 않음 -> 새 방
  (애매하면 IngestError 로 중단한다. 잘못 합치면 두 방의 기록이 섞여 되돌리기 어렵기 때문이다)
"""
from __future__ import annotations

import hashlib
from difflib import SequenceMatcher
from pathlib import Path

from . import state as st
from .parse import ParsedExport
from .reconcile import IngestError

MIN_COMPARE = 3          # 겹침을 판단하려면 비교 대상이 이 개수 이상이어야 한다
SAME_RATIO = 0.5         # 제목이 같을 때 이 비율 미만이면 의심 (같은 이름의 다른 방 등)
RENAME_RATIO = 0.6       # 제목이 다를 때 이 비율 이상이면 이름 변경 후보
OTHER_MAX_RATIO = 0.3    # 이름 변경 후보 외 다른 방은 이 비율 미만이어야 한다


def overlap(state: st.State, parsed: ParsedExport) -> tuple[int, int]:
    """(맞은 수, 비교 대상 수). 새 내보내기의 첫 날짜 이후의 이전 활성 메시지만 비교한다."""
    if not parsed.first_date:
        return 0, 0
    old = [r for i in state.visible if (r := state.registry[i])["kind"] == "message"
           and r["status"] == "active" and r["date"] >= parsed.first_date]
    new = [(e.date, e.hhmm, e.text) for e in parsed.entries if e.kind == "message"]
    sm = SequenceMatcher(None, [(r["date"], r["hhmm"], r["text"]) for r in old], new, autojunk=False)
    return sum(n for _, _, n in sm.get_matching_blocks()), len(old)


def new_conversation_id(parsed: ParsedExport) -> str:
    return "kt_" + hashlib.sha1(f"{parsed.title}|{parsed.saved_at}".encode("utf-8")).hexdigest()[:10]


def _ratio(m: int, c: int):
    return None if c == 0 else m / c


def _fmt(cid: str, m: int, c: int) -> str:
    return f"{cid}: 겹침 {m}/{c}" + ("" if c == 0 else f" ({m / c:.0%})")


def resolve(root: Path, parsed: ParsedExport, hint: str | None = None, force: bool = False) -> tuple[str, dict]:
    """(conversation_id, link) 를 돌려준다. link 는 conversation.renamed 이벤트에 근거로 남는다."""
    root = Path(root)
    cands = []
    for d in sorted(p for p in root.iterdir() if (p / "events.jsonl").exists()) if root.exists() else []:
        state, _ = st.load(d / "events.jsonl")
        m, c = overlap(state, parsed)
        cands.append({"id": d.name, "m": m, "c": c, "r": _ratio(m, c),
                      "title_match": parsed.title in {t["title"] for t in state.titles}})

    def link(status, basis, cand):
        return {"status": status, "basis": basis, "matched": cand["m"] if cand else 0,
                "compared": cand["c"] if cand else 0}

    if hint:
        cand = next((c for c in cands if c["id"] == hint), None)
        if cand and cand["c"] >= MIN_COMPARE and cand["r"] < OTHER_MAX_RATIO and not force:
            raise IngestError(
                f"--conversation {hint} 와 이 내보내기가 거의 겹치지 않는다 ({_fmt(hint, cand['m'], cand['c'])}). "
                "다른 방의 내보내기일 수 있다. 확실하면 --force")
        return hint, link("explicit", "explicit_conversation", cand)

    tm = [c for c in cands if c["title_match"]]
    if tm:
        ok = [c for c in tm if c["r"] is None or c["c"] < MIN_COMPARE or c["r"] >= SAME_RATIO]
        if len(ok) == 1:
            return ok[0]["id"], link("same", "title_match", ok[0])
        raise IngestError(
            f"제목 {parsed.title!r}과 일치하는 방을 하나로 정하지 못했다 ("
            + "; ".join(_fmt(c["id"], c["m"], c["c"]) for c in tm)
            + "). 같은 이름의 다른 방이거나 내용이 크게 바뀌었을 수 있다. --conversation 으로 지정하라")

    scored = sorted((c for c in cands if c["c"] >= MIN_COMPARE), key=lambda c: c["r"], reverse=True)
    if scored:
        best, second = scored[0], (scored[1] if len(scored) > 1 else None)
        if best["r"] >= RENAME_RATIO and best["m"] >= MIN_COMPARE and (second is None or second["r"] < OTHER_MAX_RATIO):
            return best["id"], link("renamed_room", "message_overlap", best)
        if best["r"] >= OTHER_MAX_RATIO:
            raise IngestError(
                f"제목 {parsed.title!r}은 처음 보는 이름인데 기존 방과 일부 겹친다 ("
                + "; ".join(_fmt(c["id"], c["m"], c["c"]) for c in scored[:3])
                + "). 이름이 바뀐 같은 방인지 확정할 수 없다. --conversation 으로 지정하라")
    return new_conversation_id(parsed), {"status": "new", "basis": "no_overlap", "matched": 0, "compared": 0}
