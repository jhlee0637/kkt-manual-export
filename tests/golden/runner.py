"""골든 시나리오 러너.

시나리오 폴더의 steps.json 을 순서대로 실행하고, 관찰 가능한 결과를 모은다.
- CLI 호출: kkt.cli.main(argv) 의 종료 코드, 표준 출력(JSON), 표준 오류의 오류 코드
- 파일 조작(op): 크래시 꼬리 흉내, 옛 로그 심기 등
- 최종 아카이브: 파일 목록과 sha256, 대화방별 events.jsonl 원문

구현 언어와 무관한 계약이다. 다른 언어의 구현도 같은 steps 를 실행해서 같은 결과를 내야 한다.
(자세한 규칙은 tests/golden/README.md)
"""
from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import re
import shlex
import subprocess
from pathlib import Path

from kkt.cli import main as kkt_main

_ERR_RE = re.compile(r"^\[중단:([a-z_]+)\]")


def _subst(value: str, scn_dir: Path, archive: Path) -> str:
    return value.replace("{dir}", str(scn_dir)).replace("{archive}", str(archive))


def _parse_stdout(text: str) -> list:
    text = text.strip()
    if not text:
        return []
    try:
        return [json.loads(text)]                        # status/participants 처럼 여러 줄짜리 JSON 하나
    except json.JSONDecodeError:
        return [json.loads(line) for line in text.splitlines() if line.strip()]   # ingest: 한 줄에 JSON 하나


def _result(code, out: str, err: str) -> dict:
    codes = [m.group(1) for line in err.splitlines() if (m := _ERR_RE.match(line))]
    return {"exit": code, "stdout": _parse_stdout(out), "error_codes": codes}


def run_cli_inprocess(argv: list) -> dict:
    out, err = io.StringIO(), io.StringIO()
    code = 0
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            code = kkt_main(argv)
        except SystemExit as e:                          # argparse 사용법 오류
            code = e.code if isinstance(e.code, int) else 2
    return _result(code, out.getvalue(), err.getvalue())


def run_cli_external(cmd: str, argv: list, cwd: Path) -> dict:
    """외부 실행 파일(다른 언어로 이식한 구현 등)을 서브프로세스로 호출한다. cmd 는 셸 단어 분리 규칙을 따른다."""
    root = str(Path(__file__).resolve().parents[2])
    env = {**os.environ, "PYTHONIOENCODING": "utf-8",
           "PYTHONPATH": root + os.pathsep + os.environ.get("PYTHONPATH", "")}
    p = subprocess.run(shlex.split(cmd) + list(argv), cwd=cwd, env=env, capture_output=True, timeout=120)
    return _result(p.returncode, p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace"))


def run_scenario(scn_dir: Path, work: Path, cmd: str | None = None) -> dict:
    """cmd 가 있으면 그 명령으로 CLI 를 호출하고, 없으면 이 저장소의 Python 구현을 직접 호출한다."""
    scn_dir, work = Path(scn_dir), Path(work)
    work.mkdir(parents=True, exist_ok=True)
    archive = work / "archive"
    steps = json.loads((scn_dir / "steps.json").read_text(encoding="utf-8"))["steps"]
    results = []
    for i, step in enumerate(steps):
        if "op" in step:
            path = Path(_subst(step["path"], scn_dir, archive))
            path.parent.mkdir(parents=True, exist_ok=True)
            if step["op"] == "write_file":
                path.write_text(step["text"], encoding="utf-8", newline="\n")
            elif step["op"] == "append_text":
                with path.open("a", encoding="utf-8", newline="\n") as f:
                    f.write(step["text"])
            else:
                raise ValueError(f"알 수 없는 op: {step['op']}")
            results.append({"step": i, "op": step["op"]})
            continue
        argv = [_subst(a, scn_dir, archive) for a in step["argv"]]
        res = run_cli_external(cmd, argv, work) if cmd else run_cli_inprocess(argv)
        results.append({"step": i, "argv": step["argv"], **res})
    tree, events = {}, {}
    if archive.exists():
        for p in sorted(archive.rglob("*")):
            if p.is_file():
                data = p.read_bytes()
                rel = p.relative_to(archive).as_posix()
                tree[rel] = {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}
                if p.name == "events.jsonl":
                    events[p.parent.name] = data
    return {"results": results, "tree": tree, "events": events}


def write_expected(scn_dir: Path, outcome: dict) -> None:
    exp = Path(scn_dir) / "expected"
    if exp.exists():
        for p in sorted(exp.rglob("*"), reverse=True):
            p.unlink() if p.is_file() else p.rmdir()
    (exp / "events").mkdir(parents=True)
    (exp / "results.json").write_text(
        json.dumps(outcome["results"], ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (exp / "tree.json").write_text(
        json.dumps(outcome["tree"], ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for conv, data in outcome["events"].items():
        (exp / "events" / f"{conv}.jsonl").write_bytes(data)


def load_expected(scn_dir: Path) -> dict:
    exp = Path(scn_dir) / "expected"
    return {
        "results": json.loads((exp / "results.json").read_text(encoding="utf-8")),
        "tree": json.loads((exp / "tree.json").read_text(encoding="utf-8")),
        "events": {p.stem: p.read_bytes() for p in sorted((exp / "events").glob("*.jsonl"))},
    }
