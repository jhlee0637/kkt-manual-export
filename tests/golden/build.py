"""골든 시나리오 생성기.

    python3 tests/golden/build.py            # 입력과 기대 결과를 다시 만든다 (동작을 일부러 바꿨을 때)
    python3 tests/golden/build.py --check    # 커밋된 것과 같은지 확인한다 (입력 재현성 + 기대 결과)

입력(TXT, 사진, steps.json)은 이 파일의 파이썬 코드로 결정적으로 만들어진다. 실제 대화는 쓰지 않는다.
기대 결과(expected/)는 현재 Python 구현이 낸 출력이며, 이식된 구현이 통과해야 하는 기준이다.
기대 결과가 바뀌면 git diff 로 정확히 어떤 동작이 바뀌었는지 리뷰할 수 있다.
"""
from __future__ import annotations

import json
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / "src" / "python"))
sys.path.insert(0, str(HERE))

from runner import run_scenario, write_expected, load_expected   # noqa: E402

DATE1 = "2026년 10월 2일 금요일"
DATE2 = "2026년 10월 3일 토요일"
CONV = "kt_golden"

CHAT = [
    "[민수] [오전 9:00] 안녕하세요",
    "[지영] [오전 9:01] 네 안녕하세요",
    "[민수] [오전 9:02] 회의 몇 시죠?",
    "[지영] [오전 9:03] 열 시요",
    "[민수] [오전 9:04] 알겠습니다",
]
INVITE = ".님이 민수님, 지영님을 초대했습니다."


def export_text(saved: str, blocks: list, title: str = "회의방") -> str:
    """blocks: [(날짜 헤더 문구, [줄...])]. 줄바꿈은 CRLF (카카오톡 내보내기와 같다)."""
    out = [f"{title} 님과 카카오톡 대화", f"저장한 날짜 : {saved}", ""]
    for hdr, lines in blocks:
        out.append(f"--------------- {hdr} ---------------")
        out.extend(lines)
    return "\r\n".join(out) + "\r\n"


def sub(lines, old, new):
    return [l.replace(f"[{old}]", f"[{new}]") for l in lines]


class Scn:
    def __init__(self, root: Path, name: str, description: str):
        self.dir = root / name
        self.name = name
        self.description = description
        self.steps: list = []
        if self.dir.exists():
            shutil.rmtree(self.dir)
        self.dir.mkdir(parents=True)

    def txt(self, name, saved, lines, title="회의방", enc="utf-8", bom=False, blocks=None):
        text = export_text(saved, blocks or [(DATE1, lines)], title)
        data = text.encode(enc)
        if bom:
            data = b"\xef\xbb\xbf" + data
        (self.dir / name).write_bytes(data)

    def photo(self, name, payload: bytes):
        (self.dir / "photos").mkdir(exist_ok=True)
        (self.dir / "photos" / name).write_bytes(payload)

    def cli(self, *args):
        self.steps.append({"argv": ["--archive", "{archive}", *args]})

    def ingest(self, file, conv=CONV, force=False, accept=()):
        args = (["--conversation", conv] if conv else []) + ["ingest", "{dir}/" + file]
        if force:
            args.append("--force")
        for a in accept:
            args += ["--accept-rename", a]
        self.cli(*args)

    def op(self, op, path, text):
        self.steps.append({"op": op, "path": path, "text": text})

    def finish(self):
        (self.dir / "steps.json").write_text(
            json.dumps({"description": self.description, "steps": self.steps}, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8")


def sc_basic(root):
    s = Scn(root, "basic_idempotent", "신규 반영, 새 메시지 추가, 같은 파일 재반영은 건너뜀, 같은 분 같은 내용의 두 메시지는 다른 ID")
    base = CHAT + ["[민수] [오전 9:05] 중복", "[민수] [오전 9:05] 중복"]
    s.txt("e1.txt", "2026-10-02 10:00:00", [INVITE] + base)
    s.txt("e2.txt", "2026-10-02 10:10:00", [INVITE] + base + ["[지영] [오전 9:10] 새 메시지"])
    s.ingest("e1.txt"); s.ingest("e2.txt"); s.ingest("e1.txt")
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_dup_tiebreak(root):
    s = Scn(root, "duplicate_longest_block",
            "가장 긴 일치 블록이 우선이다: 중복 메시지 뒤로 이어지는 더 긴 일치가 있으면 앞쪽(먼저 온) 중복이 missing 이 된다")
    dup = ["[민수] [오전 9:05] 중복", "[민수] [오전 9:05] 중복", "[민수] [오전 9:05] 중복"]
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT[:2] + dup + CHAT[2:])
    s.txt("e2.txt", "2026-10-02 10:10:00", CHAT[:2] + dup[:2] + CHAT[2:])      # 중복 셋 중 하나가 로컬 삭제로 사라짐
    s.txt("e3.txt", "2026-10-02 10:20:00", CHAT[:2] + dup[:1] + CHAT[2:])      # 또 하나가 사라짐
    s.ingest("e1.txt"); s.ingest("e2.txt"); s.ingest("e3.txt")
    s.finish()


def sc_tie(root):
    s = Scn(root, "tie_break_equal_runs",
            "길이가 같은 일치가 여럿이면 a(이전 기록)에서 앞쪽, 그다음 b 에서 앞쪽을 고른다: 떨어져 있는 같은 메시지 둘 중 먼저 온 것이 유지된다")
    d = "[민수] [오전 9:05] 네"
    s.txt("e1.txt", "2026-10-02 10:00:00", [d, "[지영] [오전 9:05] 알겠어요", d, "[지영] [오전 9:05] ㅇㅋ"])
    s.txt("e2.txt", "2026-10-02 10:10:00", [d, "[지영] [오전 9:20] 다른 말"])
    s.ingest("e1.txt"); s.ingest("e2.txt", force=True)      # 활성 4개 중 3개가 사라지므로 --force
    s.finish()


def sc_delete(root):
    s = Scn(root, "delete_for_everyone", "모두에게 삭제는 같은 자리의 삭제 표식으로 확정, 표식은 이후에도 유지, 사진 삭제도 같은 표식")
    base = CHAT[:2] + ["[민수] [오전 9:02] 사진"] + CHAT[2:]
    s.txt("e1.txt", "2026-10-02 10:00:00", base)
    after = list(base)
    after[2] = "메시지가 삭제되었습니다."
    after[0] = "메시지가 삭제되었습니다."
    s.txt("e2.txt", "2026-10-02 10:10:00", after)
    s.txt("e3.txt", "2026-10-02 10:20:00", after + ["[지영] [오전 9:10] 이후"])
    s.ingest("e1.txt"); s.ingest("e2.txt"); s.ingest("e3.txt")
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_missing(root):
    s = Scn(root, "local_delete_and_reappear", "나에게서만 삭제는 흔적 없이 사라짐(missing), 같은 내용이 다시 나타나면 reappeared")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT)
    s.txt("e2.txt", "2026-10-02 10:10:00", [l for l in CHAT if "열 시요" not in l])
    s.txt("e3.txt", "2026-10-02 10:20:00", CHAT)
    s.ingest("e1.txt"); s.ingest("e2.txt"); s.ingest("e3.txt")
    s.finish()


def sc_edit(root):
    s = Scn(root, "edit_candidate", "수정은 같은 위치·분·보낸이에서 내용만 바뀐 후보. 같은 분에 여럿이고 개수가 다르면 짝짓지 않음")
    base = CHAT + ["[민수] [오전 10:06] t1", "[민수] [오전 10:06] t2"]
    s.txt("e1.txt", "2026-10-02 10:10:00", base)
    s.txt("e2.txt", "2026-10-02 10:20:00", [l.replace("] t2", "] t2-2") for l in base])
    ambiguous = [l for l in base if "t1" not in l]
    s.txt("e3.txt", "2026-10-02 10:30:00", [l.replace("] t2-2", "] t2-3").replace("] t2", "] t2-3") for l in ambiguous])
    s.ingest("e1.txt"); s.ingest("e2.txt"); s.ingest("e3.txt", force=True)
    s.finish()


def sc_realworld(root):
    s = Scn(root, "combined_realworld", "실측 시나리오: 삭제+수정, 이후 로컬 삭제. 사진·이모티콘·답장(인용 없이 본문만)·여러 줄 메시지")
    base = [INVITE, "[민수] [오전 9:27] Test", "[민수] [오전 9:27] Test", "[민수] [오전 9:28] 사진",
            "[민수] [오전 9:28] 테스트 좀 할게요..", "[지영] [오전 9:33] 넵넵", "[지영] [오전 9:42] ?!",
            "[지영] [오전 9:42] 넵 알겠습니다", "[민수] [오전 10:06] test1", "[민수] [오전 10:06] test2",
            "[민수] [오전 10:07] 사진", "[민수] [오전 10:08] 사진",
            "[민수] [오전 10:39] reply", "[민수] [오전 10:39] reply", "[민수] [오전 10:39] 이모티콘",
            "[지영] [오전 10:40] 첫 줄", "둘째 줄", "", "넷째 줄"]
    s.txt("e1.txt", "2026-10-02 10:45:00", base)
    e2 = list(base)
    e2[4] = "메시지가 삭제되었습니다."                 # 모두에게 삭제
    e2[10] = "메시지가 삭제되었습니다."
    e2[9] = "[민수] [오전 10:06] test2-2"             # 수정
    s.txt("e2.txt", "2026-10-02 10:50:00", e2)
    e3 = [l for l in e2 if "넵 알겠습니다" not in l and "test2-2" not in l]   # 나에게서만 삭제
    s.txt("e3.txt", "2026-10-02 10:55:00", e3)
    for f in ("e1.txt", "e2.txt", "e3.txt"):
        s.ingest(f)
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_trim(root):
    s = Scn(root, "history_trimmed", "새 내보내기가 더 늦은 날짜부터 시작하면 사라진 항목은 삭제가 아니라 unverifiable")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT)
    s.txt("e2.txt", "2026-10-03 09:00:00", [], blocks=[(DATE2, ["[민수] [오전 9:00] 다음 날"])])
    s.ingest("e1.txt"); s.ingest("e2.txt", force=True)
    s.finish()


def sc_errors(root):
    s = Scn(root, "safety_errors", "대량 소실은 중단(--force 로 통과), 빈 내보내기는 중단, 오류 코드는 안정된 계약")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT)
    s.txt("e2.txt", "2026-10-02 10:10:00", CHAT[:2])      # 겹침 2/5=40%(방은 맞음), 활성 5개 중 3개 소실 → mass_loss
    s.txt("e3.txt", "2026-10-02 10:20:00", [])
    s.ingest("e1.txt"); s.ingest("e2.txt"); s.ingest("e3.txt"); s.ingest("e2.txt", force=True)
    s.cli("--conversation", CONV, "ingest", "{dir}/e1.txt", "--accept-rename", "형식오류")
    s.finish()


def sc_room_rename(root):
    s = Scn(root, "room_rename", "방 이름 변경: 제목이 처음이어도 메시지 겹침으로 같은 방으로 이어짐 (--conversation 없이 판정)")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT, title="구 회의방")
    s.txt("e2.txt", "2026-10-02 10:10:00", CHAT + ["[지영] [오전 9:10] 새 메시지"], title="신 회의방")
    s.txt("e3.txt", "2026-10-02 10:20:00", CHAT + ["[지영] [오전 9:10] 새 메시지", "[민수] [오전 9:11] 또"], title="신 회의방")
    for f in ("e1.txt", "e2.txt", "e3.txt"):
        s.ingest(f, conv=None)
    s.finish()


def sc_thresholds(root):
    s = Scn(root, "room_overlap_thresholds",
            "겹침 비율의 경계값: 제목이 새로울 때 정확히 60% 면 이름 변경, 제목이 같을 때 정확히 50% 면 같은 방 (둘 다 '이상'이다)")
    P = [f"[민수] [오전 9:0{i}] P{i}" for i in range(5)]
    s.txt("p1.txt", "2026-10-02 10:00:00", P, title="P방")
    # 새 제목, 겹침 3/5 = 0.6 → renamed_room
    s.txt("p2.txt", "2026-10-02 10:10:00", P[:3] + ["[지영] [오전 9:10] Pnew1", "[지영] [오전 9:11] Pnew2"], title="Q방")
    R = [f"[철수] [오전 8:0{i}] R{i}" for i in range(6)]
    s.txt("r1.txt", "2026-10-02 10:20:00", R, title="R방")
    # 같은 제목, 겹침 3/6 = 0.5 → same
    s.txt("r2.txt", "2026-10-02 10:30:00",
          R[:3] + [f"[영희] [오전 8:1{i}] Rnew{i}" for i in range(3)], title="R방")
    for f in ("p1.txt", "p2.txt", "r1.txt", "r2.txt"):
        s.ingest(f, conv=None)
    s.finish()


def sc_room_resolution(root):
    s = Scn(root, "room_resolution", "방 판정: 무관한 내용은 새 방, 같은 제목 다른 내용/일부 겹침은 중단, 같은 제목은 같은 방")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT, title="A방")
    s.txt("e2.txt", "2026-10-02 10:10:00", ["[철수] [오전 8:00] 전혀", "[영희] [오전 8:01] 다른", "[철수] [오전 8:02] 대화"], title="B방")
    s.txt("e3.txt", "2026-10-02 10:20:00", ["[철수] [오전 7:00] 또", "[영희] [오전 7:01] 다른", "[철수] [오전 7:02] 내용"], title="A방")
    s.txt("e4.txt", "2026-10-02 10:30:00", CHAT[:2] + ["[철수] [오전 9:30] x", "[철수] [오전 9:31] y", "[철수] [오전 9:32] z"], title="C방")
    s.txt("e5.txt", "2026-10-02 10:40:00", CHAT + ["[지영] [오전 9:10] 추가"], title="A방")
    for f in ("e1.txt", "e2.txt", "e3.txt", "e4.txt", "e5.txt"):
        s.ingest(f, conv=None)
    s.finish()


def sc_explicit(root):
    s = Scn(root, "explicit_conversation", "--conversation 으로 지정했지만 거의 겹치지 않으면 중단, --force 로 통과")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT)
    s.txt("e2.txt", "2026-10-02 10:10:00", ["[철수] [오전 8:00] a", "[영희] [오전 8:01] b", "[철수] [오전 8:02] c"], title="다른방")
    s.ingest("e1.txt", conv="kt_a"); s.ingest("e2.txt", conv="kt_a"); s.ingest("e2.txt", conv="kt_a", force=True)
    s.finish()


def sc_member_rename(root):
    s = Scn(root, "member_rename", "멤버 이름 변경은 소급된다(메시지+초대 시스템 줄 근거로 자동 확정). 되돌림은 restored_previous_name")
    body = ["[철수] [오전 9:00] 하나", "[민수] [오전 9:01] 둘", "[철수] [오전 9:02] 셋", "[지영] [오전 9:03] 넷"]
    orig = [".님이 철수님, 민수님, 지영님을 초대했습니다."] + body
    renamed = [".님이 철수샘님, 민수님, 지영님을 초대했습니다."] + sub(body, "철수", "철수샘")   # 과거 줄과 시스템 줄까지 소급
    s.txt("e1.txt", "2026-10-02 10:00:00", orig)
    s.txt("e2.txt", "2026-10-02 10:10:00", renamed + ["[철수샘] [오전 9:10] 새 이름으로"])
    s.txt("e3.txt", "2026-10-02 10:20:00", renamed + ["[철수샘] [오전 9:10] 새 이름으로", "[민수] [오전 9:11] 다음"])
    back = orig + ["[철수] [오전 9:10] 새 이름으로", "[민수] [오전 9:11] 다음"]                 # 되돌림도 소급
    s.txt("e4.txt", "2026-10-02 10:30:00", back)
    s.txt("e5.txt", "2026-10-02 10:40:00", back + ["[지영] [오전 9:12] 끝"])
    for f in ("e1.txt", "e2.txt", "e3.txt", "e4.txt", "e5.txt"):
        s.ingest(f)
    s.cli("--conversation", CONV, "participants")
    s.finish()


def sc_member_boundary(root):
    s = Scn(root, "member_rename_evidence_boundary",
            "자동 확정에 필요한 근거는 정확히 2건: (메시지 1 + 시스템 줄 1) 또는 (메시지 2 + 시스템 줄 없음)이면 확정")
    a1 = [".님이 가나님, 다라님을 초대했습니다.", "[가나] [오전 9:00] 하나", "[다라] [오전 9:01] 둘", "[다라] [오전 9:02] 셋"]
    a2 = [".님이 가나샘님, 다라님을 초대했습니다.", "[가나샘] [오전 9:00] 하나", "[다라] [오전 9:01] 둘", "[다라] [오전 9:02] 셋"]
    s.txt("a1.txt", "2026-10-02 10:00:00", a1, title="가방")
    s.txt("a2.txt", "2026-10-02 10:10:00", a2, title="가방")
    b1 = ["[마바] [오전 9:00] 하나", "[마바] [오전 9:01] 둘", "[사아] [오전 9:02] 셋"]
    b2 = ["[마바샘] [오전 9:00] 하나", "[마바샘] [오전 9:01] 둘", "[사아] [오전 9:02] 셋"]
    s.txt("b1.txt", "2026-10-02 10:20:00", b1, title="나방")
    s.txt("b2.txt", "2026-10-02 10:30:00", b2, title="나방")
    s.ingest("a1.txt", conv="kt_a"); s.ingest("a2.txt", conv="kt_a")
    s.ingest("b1.txt", conv="kt_b"); s.ingest("b2.txt", conv="kt_b")
    s.finish()


def sc_member_weak(root):
    s = Scn(root, "member_rename_weak", "근거가 약한 이름 변경(메시지 1건, 시스템 줄 없음)은 확정하지 않고 경고, --accept-rename 으로 확정")
    chat = ["[철수] [오전 9:00] 하나", "[민수] [오전 9:01] 둘", "[민수] [오전 9:02] 셋"]
    s.txt("e1.txt", "2026-10-02 10:00:00", chat)
    s.txt("e2.txt", "2026-10-02 10:10:00", sub(chat, "철수", "철수샘"))
    s.ingest("e1.txt"); s.ingest("e2.txt", force=True)
    s.cli("--conversation", CONV, "ingest", "{dir}/e2.txt", "--accept-rename", "철수=철수샘")   # 이미 반영됨 → 건너뜀
    s.finish()
    s2 = Scn(root, "member_rename_accept", "같은 입력을 처음부터 --accept-rename 과 함께 반영하면 forced_by_user 로 확정")
    s2.txt("e1.txt", "2026-10-02 10:00:00", chat)
    s2.txt("e2.txt", "2026-10-02 10:10:00", sub(chat, "철수", "철수샘"))
    s2.ingest("e1.txt"); s2.ingest("e2.txt", accept=["철수=철수샘"])
    s2.cli("--conversation", CONV, "participants")
    s2.finish()


def sc_nonretro(root):
    s = Scn(root, "non_retroactive_manual_link", "새 이름이 새 메시지에만 나타나면 새 참가자. 수동 연결 뒤에는 옛 이름이 계속 찍혀도 경고·이벤트 없음")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT)
    s.txt("e2.txt", "2026-10-02 10:10:00", CHAT + ["[민수2] [오전 9:10] 이름 바꿨어요"])
    s.txt("e3.txt", "2026-10-02 10:20:00", CHAT + ["[민수2] [오전 9:10] 이름 바꿨어요", "[지영] [오전 9:12] 끝"])
    s.ingest("e1.txt"); s.ingest("e2.txt")
    s.cli("--conversation", CONV, "participant-link", "--keep", "민수", "--merge", "민수2")
    s.cli("--conversation", CONV, "participant-link", "--keep", "민수", "--merge", "없는사람")
    s.ingest("e3.txt")
    s.cli("--conversation", CONV, "participants")
    s.finish()


def sc_format(root):
    s = Scn(root, "encoding_and_format", "UTF-8 BOM, CP949, 오전/오후 12시, 여러 줄 메시지, 날짜 헤더가 둘, 이모티콘")
    day1 = ["[민수] [오전 12:05] 자정", "[지영] [오후 12:05] 정오", "[민수] [오후 11:59] 늦은 밤",
            "[지영] [오전 9:00] 첫 줄", "둘째 줄", "", "넷째 줄", "[민수] [오전 9:01] 이모티콘"]
    day2 = ["[지영] [오전 12:01] 다음 날 자정 지나서"]
    s.txt("e1.txt", "2026-10-03 08:00:00", [], bom=True, blocks=[(DATE1, day1), (DATE2, day2)])
    s.txt("e2.txt", "2026-10-03 09:00:00", [], enc="cp949",
          blocks=[(DATE1, day1), (DATE2, day2 + ["[민수] [오전 8:30] 아침"])])
    s.ingest("e1.txt"); s.ingest("e2.txt")
    s.finish()


def sc_exotic_separators(root):
    s = Scn(root, "exotic_line_separators",
            "줄 구분은 \\r\\n, \\n, \\r 만이다. 본문의 U+2028, U+0085, \\x0b 등은 본문의 일부로 보존된다 (로그를 다시 읽어도 안 깨짐)")
    body = ["[민수] [오전 9:00] 앞\u2028뒤\u0085끝\x0b!", "[지영] [오전 9:01] 보통 메시지"]
    s.txt("e1.txt", "2026-10-02 10:00:00", body)
    s.txt("e2.txt", "2026-10-02 10:10:00", body + ["[민수] [오전 9:02] 이어서"])
    s.ingest("e1.txt"); s.ingest("e2.txt")
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_attach(root):
    s = Scn(root, "attachments_link", "사진 연결: 같은 분의 사진 수와 파일 수가 같을 때만 시간순 연결, 중복 저장본은 alias, 남의 파일은 무시")
    png = b"\x89PNG\r\n\x1a\n"
    s.photo("KakaoTalk_20261002_092801000.png", png + b"golden-A")
    s.photo("KakaoTalk_20261002_092830000.png", png + b"golden-B")
    s.photo("KakaoTalk_20261002_100000100.png", png + b"golden-C")
    s.photo("KakaoTalk_20261002_100000100 (1).png", png + b"golden-C")
    s.photo("KakaoTalk_20261002_110000000.png", png + b"golden-D")
    s.photo("KakaoTalk_20261002_110005000.png", png + b"golden-E")
    (s.dir / "photos" / "report.pdf").write_bytes(b"%PDF-1.4 not a kakao photo")
    s.txt("e1.txt", "2026-10-02 12:00:00", ["[민수] [오전 9:28] 사진", "[민수] [오전 9:28] 사진",
                                              "[지영] [오전 10:00] 사진", "[민수] [오전 11:00] 사진"])
    s.ingest("e1.txt")
    s.cli("--conversation", CONV, "attach", "{dir}/photos")
    s.cli("--conversation", CONV, "attach", "{dir}/photos")
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_crash(root):
    s = Scn(root, "crash_tail_recovery", "종결 이벤트 없는 로그 꼬리(크래시)는 읽을 때 무시하고 다음 쓰기에서 잘라냄")
    s.txt("e1.txt", "2026-10-02 10:00:00", CHAT)
    s.txt("e2.txt", "2026-10-02 10:10:00", CHAT + ["[지영] [오전 9:10] 이후"])
    s.ingest("e1.txt")
    s.op("append_text", "{archive}/" + CONV + "/events.jsonl",
         json.dumps({"type": "message.observed", "event_id": "ev_999999", "schema_version": 2}) + "\n")
    s.cli("--conversation", CONV, "status")
    s.ingest("e2.txt")
    s.finish()


def sc_schema(root):
    s = Scn(root, "old_schema_rejected", "스키마 v1 로그는 조용히 잘못 읽지 않고 schema_too_old 로 거부")
    s.op("write_file", "{archive}/" + CONV + "/events.jsonl",
         json.dumps({"type": "export.ingested", "event_id": "ev_000001", "schema_version": 1, "visible": [],
                     "export_name": "x", "export_sha256": "x", "saved_at": "x"}) + "\n")
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_attach_multi(root):
    s = Scn(root, "attachments_multi",
            "사진 여러 장을 한 번에 보내면 '사진 N장' 한 줄이다. 같은 분의 사진 수 합계(N 의 합)와 파일 수가 같을 때만 메시지 순서대로 N장씩 연결하고, 다르면 연결하지 않고 photos 와 함께 보고한다. 같은 사진을 다른 시각에 다시 보낸 것은 별개 첨부이고, ' (1)' 재저장본만 별칭이다")
    png = b"\x89PNG\r\n\x1a\n"
    # 9:00 에 4장: 앞 메시지(사진 3장)는 같은 시각의 묶음이라 _01, _02 가 붙고, 다음 메시지(사진 1장)는 다른 시각
    s.photo("KakaoTalk_20261002_090000123.png", png + b"multi-0")
    s.photo("KakaoTalk_20261002_090000123_01.png", png + b"multi-1")
    s.photo("KakaoTalk_20261002_090000123_02.png", png + b"multi-2")
    s.photo("KakaoTalk_20261002_090030000.png", png + b"multi-3")
    s.photo("KakaoTalk_20261002_100000000.png", png + b"lonely")                            # 10:00 에 1장 (메시지는 2장)
    s.photo("KakaoTalk_20261002_110000000.png", png + b"pair-a")
    s.photo("KakaoTalk_20261002_110030000.png", png + b"pair-b")                            # 11:00 에 2장
    s.photo("KakaoTalk_20261002_113000000.png", png + b"pair-a")                            # 11:30 에 같은 사진을 다시 보냄 (내용이 11:00 의 한 장과 같다)
    s.photo("KakaoTalk_20261002_113000000 (1).png", png + b"pair-a")                        # 같은 파일의 재저장본은 별칭
    s.photo("KakaoTalk_20261002_091005000.mp4", b"\x00\x00\x00\x18ftypmp42" + b"video-A")       # 9:10 동영상 (메시지 1개, 파일 1개)
    s.txt("e1.txt", "2026-10-02 12:00:00", ["[민수] [오전 9:00] 사진 3장", "[지영] [오전 9:00] 사진",
                                              "[민수] [오전 10:00] 사진 2장", "[민수] [오전 11:00] 사진 2장", "[민수] [오전 11:30] 사진", "[민수] [오전 9:10] 동영상", "[지영] [오전 9:20] 동영상",
                                              "[지영] [오전 11:05] 사진 찍었어", "[지영] [오전 11:06] 사진 0장"])
    s.ingest("e1.txt")
    s.cli("--conversation", CONV, "attach", "{dir}/photos")
    s.cli("--conversation", CONV, "attach", "{dir}/photos")
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_attach_no_videos(root):
    s = Scn(root, "attachments_no_videos",
            "동영상 보관을 끄면(--no-videos) 사진만 보관·연결하고 .mp4 는 보관도 연결도 하지 않는다. 끄지 않고 다시 하면 그때 보관·연결한다")
    png = b"\x89PNG\r\n\x1a\n"
    s.photo("KakaoTalk_20261002_090000000.png", png + b"photo-only")
    s.photo("KakaoTalk_20261002_090500000.mp4", b"\x00\x00\x00\x18ftypmp42" + b"video-only")
    s.txt("e1.txt", "2026-10-02 12:00:00", ["[민수] [오전 9:00] 사진", "[민수] [오전 9:05] 동영상"])
    s.ingest("e1.txt")
    s.cli("--conversation", CONV, "attach", "{dir}/photos", "--no-videos")
    s.cli("--conversation", CONV, "status")
    s.cli("--conversation", CONV, "attach", "{dir}/photos")
    s.cli("--conversation", CONV, "status")
    s.finish()


def sc_attach_saved_first(root):
    s = Scn(root, "attachments_saved_first",
            "파일을 먼저 보관하고 나중에 텍스트를 반영해도(보관한 첨부의 종류는 mime 으로 읽는다) 같은 분의 사진은 사진끼리, 동영상은 동영상끼리 연결된다")
    png = b"\x89PNG\r\n\x1a\n"
    s.photo("KakaoTalk_20261002_090000000.png", png + b"early-photo")
    s.photo("KakaoTalk_20261002_090000500.mp4", b"\x00\x00\x00\x18ftypmp42" + b"early-video")   # 같은 분의 동영상
    s.txt("e1.txt", "2026-10-02 12:00:00", ["[민수] [오전 9:00] 동영상", "[민수] [오전 9:00] 사진"])
    s.cli("--conversation", CONV, "attach", "{dir}/photos")      # 아직 메시지가 없다: 보관만
    s.ingest("e1.txt")
    s.cli("--conversation", CONV, "attach", "{dir}/photos")      # 이제 연결한다
    s.cli("--conversation", CONV, "status")
    s.finish()


SCENARIOS = [sc_basic, sc_thresholds, sc_dup_tiebreak, sc_tie, sc_delete, sc_missing, sc_edit, sc_realworld, sc_trim, sc_errors, sc_room_rename,
             sc_room_resolution, sc_explicit, sc_member_rename, sc_member_boundary, sc_member_weak, sc_nonretro, sc_format,
             sc_exotic_separators, sc_attach, sc_attach_multi, sc_attach_no_videos, sc_attach_saved_first, sc_crash, sc_schema]


def build(root: Path) -> list:
    for fn in SCENARIOS:
        fn(root)
    return sorted(p for p in root.iterdir() if p.is_dir() and (p / "steps.json").exists())


def snapshot(d: Path) -> dict:
    return {p.relative_to(d).as_posix(): p.read_bytes() for p in sorted(d.rglob("*")) if p.is_file()}


def main(argv=None) -> int:
    check = "--check" in (argv or sys.argv[1:])
    target = HERE / "scenarios"
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "scenarios"
        root.mkdir()
        dirs = build(root)
        for d in dirs:
            with tempfile.TemporaryDirectory() as work:
                write_expected(d, run_scenario(d, Path(work)))
        if check:
            if snapshot(root) != snapshot(target):
                a, b = snapshot(root), snapshot(target)
                diff = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
                print("골든이 최신이 아니다:", *diff[:20], sep="\n  ")
                return 1
            print(f"골든 {len(dirs)}개 시나리오가 최신이다")
            return 0
        if target.exists():
            shutil.rmtree(target)
        shutil.copytree(root, target)
        print(f"골든 {len(dirs)}개 시나리오를 {target} 에 만들었다")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
