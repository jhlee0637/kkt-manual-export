"""골든 시나리오를 임의의 구현으로 실행하고 어긋나는 지점을 보여 준다 (이식 작업용).

    python3 tests/golden/run.py                                  # 이 저장소의 Python 구현 (직접 호출)
    python3 tests/golden/run.py --cmd "python3 -m kkt"           # 서브프로세스로 호출
    python3 tests/golden/run.py --cmd "rust/target/release/kkt"  # 이식한 구현
    python3 tests/golden/run.py --cmd ... combined_realworld     # 시나리오 이름으로 일부만
    KKT_GOLDEN_CMD="..." python3 tests/golden/run.py             # 명령을 환경 변수로

종료 코드: 모두 통과 0, 하나라도 실패 1.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1]))
sys.path.insert(0, str(HERE))

from runner import load_expected, run_scenario   # noqa: E402

SCENARIOS = HERE / "scenarios"


def _short(x, n=160) -> str:
    t = json.dumps(x, ensure_ascii=False, sort_keys=True) if not isinstance(x, str) else x
    return t if len(t) <= n else t[:n] + "…"


def diff_outcome(out: dict, exp: dict) -> list:
    """어긋난 곳을 사람이 읽을 수 있게 요약한다. 비어 있으면 같다."""
    problems = []
    got = json.loads(json.dumps(out["results"], ensure_ascii=False))
    if len(got) != len(exp["results"]):
        problems.append(f"단계 수가 다르다: 기대 {len(exp['results'])}, 실제 {len(got)}")
    for i, (a, b) in enumerate(zip(got, exp["results"])):
        if a != b:
            for k in sorted(set(a) | set(b)):
                if a.get(k) != b.get(k):
                    problems.append(f"단계 {i} `{k}`:\n      기대 {_short(b.get(k))}\n      실제 {_short(a.get(k))}")
    for rel in sorted(set(out["tree"]) | set(exp["tree"])):
        if out["tree"].get(rel) != exp["tree"].get(rel):
            state = "없음" if rel not in out["tree"] else ("예상 밖" if rel not in exp["tree"] else "내용이 다름")
            problems.append(f"파일 {rel}: {state}")
    for conv in sorted(set(out["events"]) | set(exp["events"])):
        a, b = out["events"].get(conv), exp["events"].get(conv)
        if a == b:
            continue
        if a is None or b is None:
            problems.append(f"events/{conv}.jsonl: {'없음' if a is None else '예상 밖'}")
            continue
        la, lb = a.decode("utf-8").splitlines(), b.decode("utf-8").splitlines()
        n = next((i for i in range(min(len(la), len(lb))) if la[i] != lb[i]), min(len(la), len(lb)))
        problems.append(f"events/{conv}.jsonl: {n + 1}번째 줄부터 다르다 (기대 {len(lb)}줄, 실제 {len(la)}줄)\n"
                        f"      기대 {_short(lb[n]) if n < len(lb) else '(없음)'}\n"
                        f"      실제 {_short(la[n]) if n < len(la) else '(없음)'}")
    return problems


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("names", nargs="*", help="시나리오 이름 (생략하면 전부)")
    ap.add_argument("--cmd", default=os.environ.get("KKT_GOLDEN_CMD"), help="CLI 실행 명령 (생략하면 Python 직접 호출)")
    a = ap.parse_args(argv)
    dirs = sorted(p for p in SCENARIOS.iterdir() if (p / "steps.json").exists())
    if a.names:
        unknown = set(a.names) - {d.name for d in dirs}
        if unknown:
            print("알 수 없는 시나리오:", ", ".join(sorted(unknown)), file=sys.stderr)
            return 2
        dirs = [d for d in dirs if d.name in a.names]
    print(f"구현: {a.cmd or 'Python 직접 호출'}   시나리오 {len(dirs)}개")
    failed = 0
    for d in dirs:
        with tempfile.TemporaryDirectory() as work:
            try:
                problems = diff_outcome(run_scenario(d, Path(work), a.cmd), load_expected(d))
            except Exception as e:                       # noqa: BLE001  실행 파일이 없거나 죽은 경우 등
                problems = [f"실행 실패: {type(e).__name__}: {e}"]
        print(f"  {'통과' if not problems else '실패'}  {d.name}")
        for p in problems[:6]:
            print("    - " + p)
        if len(problems) > 6:
            print(f"    … 외 {len(problems) - 6}건")
        failed += bool(problems)
    print(f"\n{len(dirs) - failed}/{len(dirs)} 통과")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
