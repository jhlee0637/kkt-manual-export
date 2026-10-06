"""새 내보내기를 이전 상태와 비교해 이벤트를 만든다.

카카오톡 내보내기에는 안정적인 메시지 ID가 없다. 그래서 직전 내보내기에 보인 항목들과
순서 기준(SequenceMatcher)으로 맞춘다. 관측된 사실만 확정으로 기록하고, 나머지는 후보/미확인으로 둔다.

판정 (test 방에서 실측한 동작에 근거):
- '모두에게 삭제'  -> 같은 자리에 '메시지가 삭제되었습니다.' 줄이 남는다.  => deleted_for_everyone (확정)
- '나에게서만 삭제' -> 줄이 아예 사라진다. 삭제 주체를 알 수 없다.        => missing (사유 불명)
- 수정             -> 표시 없이 같은 시각/보낸이로 내용만 바뀐다.         => edit_candidate (후보)
"""
from __future__ import annotations

import hashlib
from collections import Counter, defaultdict
from difflib import SequenceMatcher

from .parse import Entry, ParsedExport
from .state import State, ident_of, key_of, relabel_text


class IngestError(Exception):
    """처리를 중단해야 하는 오류. message 는 사람용(한국어), code 는 구현이 바뀌어도 변하지 않는 계약이다."""

    def __init__(self, message: str, code: str = "ingest_error"):
        super().__init__(message)
        self.code = code


def _iso(date: str, hhmm: str) -> str:
    return f"{date}T{hhmm}:00+09:00"


def _content(e: Entry) -> list:
    if e.content_type == "image":
        return [{"type": "image", "attachment_id": None} for _ in range(e.image_count)]
    if e.content_type == "emoticon":
        return [{"type": "emoticon"}]
    if e.content_type == "video":
        return [{"type": "video"}]
    return [{"type": "text", "text": e.text}]


def _neighbors(entries: list, j: int) -> dict:
    """삭제 표식처럼 시각이 없는 줄의 시각 범위를 앞뒤 메시지로 추정한다."""
    prev = next((_iso(e.date, e.hhmm) for e in reversed(entries[:j]) if e.hhmm), None)
    nxt = next((_iso(e.date, e.hhmm) for e in entries[j + 1:] if e.hhmm), None)
    return {"inferred_after": prev, "inferred_before": nxt}


MIN_RENAME_EVIDENCE = 2    # 자동 확정에 필요한 '같은 시각/내용인데 이름만 다른' 메시지 수


def _ident_entry(e: Entry, idx: dict):
    return idx.get(e.sender) or ("n", e.sender)


def entry_key(e: Entry, idx: dict) -> tuple:
    """state.key_of 와 같은 모양. 발신자는 이름이 아니라 참가자로 환산해서 비교한다."""
    if e.kind == "message":
        return ("m", e.date, e.hhmm, _ident_entry(e, idx), e.text)
    if e.kind == "deleted_marker":
        return ("d", e.date)
    return ("s", e.date, e.text)


def detect_renames(state: State, parsed: ParsedExport, accept: dict | None = None) -> tuple[list, list]:
    """멤버 이름 변경을 찾는다. (확정 목록, 확정하지 못한 의심 목록).

    카카오톡이 이름 변경 뒤 내보낼 때 과거 메시지의 이름도 새 이름으로 찍는지는 실측 전이다.
    그런 경우를 위해, 시각/내용이 같은 메시지끼리 맞췄을 때 발신자 이름만 일관되게 달라졌다면
    이름 변경으로 본다. 근거가 약하면 확정하지 않고 의심으로 남긴다 (accept 로 사람이 확인하면 확정).
    (과거 메시지는 옛 이름 그대로이고 새 이름이 새 메시지에만 나타나는 경우는 여기서 알 수 없다.
     TXT 에 근거가 없으므로 수동 연결(participant-link)을 쓴다.)

    확정 조건 (모두 만족):
      - 그 참가자의 정렬된 메시지가 전부 같은 새 이름 하나로만 바뀌었다
      - 그 참가자의 어떤 이름도 새 내보내기의 발신자로 더 이상 나오지 않는다 (변경이 완결됨)
      - 새 이름이 이 방의 다른 참가자가 쓰던 이름이 아니다 (두 사람이 합쳐지는 것을 막는다)
      - 서로 다른 참가자 둘이 같은 새 이름으로 가지 않는다
      - 근거가 MIN_RENAME_EVIDENCE 건 이상 (또는 accept 로 확인됨).
        근거 = 이름만 달라진 정렬된 메시지 수 + 같은 치환이 일어난 시스템 줄 수
    """
    accept = accept or {}
    idx = state.name_index()
    cur = state.current_names()
    old = [r for i in state.visible if (r := state.registry[i])["kind"] == "message"
           and r["status"] == "active" and r.get("participant_id")]
    new = [e for e in parsed.entries if e.kind == "message"]
    sm = SequenceMatcher(None, [(r["date"], r["hhmm"], r["text"]) for r in old],
                         [(e.date, e.hhmm, e.text) for e in new], autojunk=False)
    per_pid: dict = defaultdict(Counter)       # pid -> {새 내보내기에 찍힌 이름: 정렬된 메시지 수}
    for i, j, n in sm.get_matching_blocks():
        for k in range(n):
            per_pid[old[i + k]["participant_id"]][new[j + k].sender] += 1
    # 두 번째 근거: 시스템 줄 본문의 이름도 같은 치환이 일어났는가 (실측: 이름 변경은 시스템 줄에도 소급된다)
    old_sys = [r for i in state.visible if (r := state.registry[i])["kind"] == "system"]
    new_sys = Counter(e.text for e in parsed.entries if e.kind == "system")
    new_senders = {e.sender for e in new}
    # 이 참가자의 정렬된 메시지가 현재 이름이 아닌 이름 하나로만 찍혔다면 이름이 바뀐 것이다.
    # 처음 보는 이름이면 '이름 변경', 이 사람이 전에 쓰던 이름이면 '되돌림'이다 (둘 다 소급된다).
    target_of = {pid: next(iter(ctr)) for pid, ctr in per_pid.items() if len(ctr) == 1 and next(iter(ctr)) != cur[pid]}
    by_target = Counter(target_of.values())
    confirmed, suspects = [], []
    for pid, ctr in per_pid.items():
        # 이 사람이 모르는 이름(처음 보는 이름이거나 다른 참가자의 이름)이 나타나지 않았고, 하나의 옛 이름으로
        # 통째로 바뀐 것도 아니면 정상이다. 한 사람의 메시지가 옛 이름과 새 이름으로 섞여 찍히는 것(비소급)도 정상이다.
        unknown = {name for name in ctr if idx.get(name) != pid}
        if not unknown and not (len(ctr) == 1 and next(iter(ctr)) != cur[pid]):
            continue
        n = target_of.get(pid)                              # 여러 이름으로 갈라졌으면 None
        o = cur[pid]
        owns = n is not None and idx.get(n) == pid          # 이 사람이 전에 쓰던 이름으로의 변경(되돌림)
        c = ctr[n] if n is not None else 0
        sys_ev = 0
        if n is not None:
            for r in old_sys:
                t = relabel_text(r["text"], o, n)
                if t != r["text"] and new_sys[t] > 0:
                    sys_ev += 1
        msg_ev, c = c, c + sys_ev
        problems = []
        if n is None:
            problems.append("이 사람의 메시지가 여러 이름으로 갈라졌다")
        if any(name in new_senders for name in state.participants[pid]["names"] if name != n):
            problems.append("옛 이름이 새 내보내기에도 발신자로 남아 있다")
        if n is not None and idx.get(n) not in (None, pid):
            problems.append("새 이름이 이미 이 방의 다른 참가자 이름이다")
        if n is not None and by_target[n] > 1:
            problems.append("서로 다른 참가자 둘이 같은 새 이름으로 간다")
        weak = n is not None and c < MIN_RENAME_EVIDENCE
        forced = n is not None and accept.get(o) == n
        item = {"old_name": o, "new_name": n, "matched": c, "problems": problems, "weak": weak,
                "evidence": {"messages": msg_ev, "system_lines": sys_ev}}
        if not problems and (not weak or forced):
            confirmed.append({**item, "participant_id": pid,
                              "basis": "forced_by_user" if forced else
                                       ("restored_previous_name" if owns else "consistent_relabel")})
        elif not owns:
            suspects.append(item)
        # owns 인데 확정하지 못한 경우는 조용히 둔다. 비소급 변경(과거 메시지는 옛 이름, 새 메시지는 새 이름)에서는
        # 옛 메시지가 늘 옛 이름으로 찍히므로 이것은 정상 상태다. 경고를 내면 매번 소음이 된다.
    return confirmed, suspects


def reconcile(state: State, parsed: ParsedExport, *, conversation_id: str,
              export_name: str, export_sha256: str, force: bool = False,
              accept_renames: dict | None = None, link: dict | None = None) -> list:
    """이벤트 목록을 반환한다 (event_id는 호출자가 부여). 상태는 바꾸지 않는다."""
    new = parsed.entries
    if not new and state.visible:
        raise IngestError("새 내보내기에 항목이 없다. 빈 파일이거나 형식이 바뀌었을 수 있다.", "empty_export")

    reg = state.registry
    observed_at = parsed.saved_at
    used: set = set()
    events: list = []

    def new_id(prefix: str, *parts: str) -> str:
        n = 0
        while True:
            h = hashlib.sha1("|".join([conversation_id, *parts, str(n)]).encode()).hexdigest()[:16]
            cid = f"{prefix}_{h}"
            if cid not in reg and cid not in used:
                used.add(cid)
                return cid
            n += 1

    def emit(type_: str, **kw):
        events.append({"type": type_, "conversation_id": conversation_id,
                       "observed_at": observed_at, "export_sha256": export_sha256, **kw})

    # 방 제목 이력
    if parsed.title:
        if state.title is None:
            emit("conversation.title_observed", title=parsed.title)
        elif parsed.title != state.title:
            emit("conversation.renamed", **{"from": state.title, "to": parsed.title},
                 basis=(link or {}).get("basis", "explicit_conversation"), link=link or {})

    # 멤버 이름 변경
    confirmed, suspects = detect_renames(state, parsed, accept_renames)
    idx = state.name_index()
    for r in confirmed:
        emit("participant.renamed", participant_id=r["participant_id"], to=r["new_name"],
             matched_messages=r["matched"], evidence=r["evidence"], basis=r["basis"],
             **{"from": r["old_name"]})
        idx[r["new_name"]] = r["participant_id"]
    warnings = list(parsed.warnings)
    for sp in suspects:
        warnings.append(
            f"발신자 이름 변경 의심 {sp['old_name']!r} -> {sp['new_name']!r} (근거 {sp['matched']}건)을 "
            f"확정하지 않았다: {'; '.join(sp['problems']) or '근거가 부족하다'}. "
            f"맞다면 --accept-rename '{sp['old_name']}={sp['new_name']}' 로 확인하라")

    def pid_for(name):
        if name in idx:
            return idx[name]
        pid = new_id("kp", "participant", name)
        idx[name] = pid
        emit("participant.observed", participant_id=pid, name=name)
        return pid

    renames = [(r["old_name"], r["new_name"]) for r in confirmed]

    def old_key(rec):
        if rec["kind"] == "system":                     # 폴드 전이라 기록에는 아직 옛 이름이 남아 있다
            t = rec["text"]
            for o, n in renames:
                t = relabel_text(t, o, n)
            return ("s", rec["date"], t)
        return key_of(rec)

    old_ids = list(state.visible)
    old_keys = [old_key(reg[i]) for i in old_ids]
    new_keys = [entry_key(e, idx) for e in new]
    assigned: list = [None] * len(new)
    vanished: list = []

    for tag, i1, i2, j1, j2 in SequenceMatcher(None, old_keys, new_keys, autojunk=False).get_opcodes():
        if tag == "equal":
            for k in range(i2 - i1):
                assigned[j1 + k] = old_ids[i1 + k]
            continue
        _block(reg, old_ids[i1:i2], list(range(j1, j2)), new, assigned, vanished, emit, new_id, observed_at,
               idx, pid_for)

    # 사라진 항목
    for oid in vanished:
        covered = bool(parsed.first_date) and reg[oid]["date"] >= parsed.first_date
        emit("message.missing", message_id=oid, range_covered=covered,
             basis="absent_in_export")

    n_active = sum(1 for i in old_ids if reg[i]["kind"] == "message" and reg[i]["status"] == "active")
    n_lost = sum(1 for i in vanished if reg[i]["kind"] == "message" and reg[i]["status"] == "active")
    if not force and n_active >= 4 and n_lost * 2 > n_active:
        raise IngestError(
            f"활성 메시지 {n_active}개 중 {n_lost}개가 사라졌다. 이 PC의 대화 내역이 지워졌거나 "
            "내보내기 범위가 달라졌을 수 있다. 확인 후 --force로 다시 실행.",
            "mass_loss",
        )

    events.append({
        "type": "export.ingested", "conversation_id": conversation_id,
        "observed_at": observed_at, "export_sha256": export_sha256,
        "export_name": export_name, "saved_at": parsed.saved_at,
        "first_date": parsed.first_date, "last_date": parsed.last_date,
        "visible": assigned, "warnings": warnings,
    })
    return events


def _block(reg, old_block, new_block, new, assigned, vanished, emit, new_id, observed_at, idx, pid_for):
    """equal이 아닌 구간 하나를 처리한다."""
    old_left = list(old_block)
    new_left = list(new_block)

    # 1) 수정 후보: 같은 날짜/분/보낸이이고 내용만 다른 쌍. 양쪽 개수가 같을 때만 순서대로 짝짓는다.
    g_old, g_new = defaultdict(list), defaultdict(list)
    for oid in old_left:
        r = reg[oid]
        if r["kind"] == "message" and r["status"] == "active":
            g_old[(r["date"], r["hhmm"], ident_of(r))].append(oid)
    for j in new_left:
        e = new[j]
        if e.kind == "message":
            g_new[(e.date, e.hhmm, _ident_entry(e, idx))].append(j)
    for trip, olist in g_old.items():
        nlist = g_new.get(trip, [])
        if len(olist) != len(nlist):
            continue
        for oid, j in zip(olist, nlist):
            assigned[j] = oid
            old_left.remove(oid)
            new_left.remove(j)
            if reg[oid]["text"] != new[j].text:
                emit("message.edit_candidate", message_id=oid,
                     previous_text=reg[oid]["text"], current_text=new[j].text,
                     basis="same_position_minute_sender", confidence="candidate")

    # 2) 모두에게 삭제: 새 삭제 표식과 사라진 활성 메시지를 순서대로 짝짓는다.
    dels = [j for j in new_left if new[j].kind == "deleted_marker"]
    olds = [oid for oid in old_left if reg[oid]["kind"] == "message" and reg[oid]["status"] == "active"]
    for j, oid in zip(dels, olds):
        assigned[j] = oid
        new_left.remove(j)
        old_left.remove(oid)
        emit("message.deleted_for_everyone", message_id=oid, basis="deleted_marker_in_place",
             **_neighbors(new, j))

    # 3) 남은 이전 항목은 사라진 것
    vanished.extend(old_left)

    # 4) 남은 새 항목은 새로 관측된 것 (이전에 사라졌던 같은 내용이면 다시 나타난 것)
    for j in new_left:
        e: Entry = new[j]
        back = next((i for i, r in reg.items()
                     if r["status"] in ("missing", "unverifiable") and key_of(r) == entry_key(e, idx)
                     and i not in assigned), None)
        if back is not None:
            assigned[j] = back
            emit("message.reappeared", message_id=back)
            continue
        if e.kind == "message":
            mid = new_id("kmsg", e.date, e.hhmm, e.sender, e.text)
            emit("message.observed", message_id=mid, kind="message", date=e.date, hhmm=e.hhmm,
                 sender=e.sender, participant_id=pid_for(e.sender), text=e.text, content_type=e.content_type,
                 timestamp=_iso(e.date, e.hhmm), timestamp_precision="minute", ordinal=j,
                 content=_content(e),
                 **({"image_count": e.image_count} if e.content_type == "image" and e.image_count > 1 else {}),
                 # TXT에는 답장 정보가 없다. 답장이 아니라는 뜻이 아니라 '알 수 없다'는 뜻이다.
                 reply_to={"status": "unknown_from_txt"})
        elif e.kind == "deleted_marker":
            mid = new_id("kdel", e.date, str(j))
            emit("deleted_marker.observed", message_id=mid, kind="deleted_marker", date=e.date,
                 text=e.text, timestamp=None, timestamp_precision="inferred", ordinal=j,
                 note="삭제 표식만 관측됨. 삭제 전 내용은 알 수 없다.", **_neighbors(new, j))
        else:
            mid = new_id("ksys", e.date, e.text)
            emit("system.observed", message_id=mid, kind="system", date=e.date, text=e.text,
                 ordinal=j, timestamp=None, timestamp_precision="inferred", **_neighbors(new, j))
        assigned[j] = mid
