"""골든 시나리오: CLI 수준의 입력→출력 계약. 이식된 구현도 같은 시나리오를 통과해야 한다."""
import json
import os
import sys
from pathlib import Path

import pytest

GOLDEN = Path(__file__).resolve().parents[1] / "golden"
sys.path.insert(0, str(GOLDEN))

from runner import load_expected, run_scenario   # noqa: E402

SCENARIOS = sorted(p for p in (GOLDEN / "scenarios").iterdir() if (p / "steps.json").exists())


def _norm(x):
    return json.loads(json.dumps(x, ensure_ascii=False))


# 서브프로세스 호출 경로(러너 자체)를 확인하는 대표 시나리오: 파일, 오류 코드, 여러 형식의 표준 출력을 모두 포함
HARNESS_SAMPLE = ["combined_realworld", "attachments_link", "room_resolution", "old_schema_rejected"]


@pytest.mark.parametrize("name", HARNESS_SAMPLE)
def test_external_command_path_gives_same_result(name, tmp_path):
    """같은 Python 구현을 `python -m kkt` 서브프로세스로 호출해도 결과가 같다. 이식된 구현을 붙이기 전에 러너가 믿을 만한지 확인한다."""
    scn = GOLDEN / "scenarios" / name
    out = run_scenario(scn, tmp_path, cmd=f"{sys.executable} -m kkt")
    exp = load_expected(scn)
    assert _norm(out["results"]) == exp["results"] and out["tree"] == exp["tree"] and out["events"] == exp["events"]


@pytest.mark.skipif(not os.environ.get("KKT_GOLDEN_CMD"), reason="KKT_GOLDEN_CMD 가 없다 (이식된 구현을 시험할 때 지정)")
@pytest.mark.parametrize("scn", SCENARIOS, ids=lambda p: p.name)
def test_external_implementation_matches_expected(scn, tmp_path):
    out, exp = run_scenario(scn, tmp_path, cmd=os.environ["KKT_GOLDEN_CMD"]), load_expected(scn)
    assert _norm(out["results"]) == exp["results"]
    assert out["tree"] == exp["tree"]
    assert out["events"] == exp["events"]


@pytest.mark.parametrize("scn", SCENARIOS, ids=lambda p: p.name)
def test_scenario_matches_expected(scn, tmp_path):
    out, exp = run_scenario(scn, tmp_path), load_expected(scn)
    assert _norm(out["results"]) == exp["results"]
    assert out["tree"] == exp["tree"]
    assert out["events"] == exp["events"]          # events.jsonl 은 바이트 단위로 같아야 한다


def test_every_scenario_replays_deterministically(tmp_path):
    """같은 시나리오를 두 번 돌려도 바이트까지 같다 (시각·무작위·경로가 새지 않는다)."""
    for scn in SCENARIOS:
        a = run_scenario(scn, tmp_path / "a" / scn.name)
        b = run_scenario(scn, tmp_path / "b" / scn.name)
        assert a["events"] == b["events"] and a["tree"] == b["tree"], scn.name


def test_committed_golden_is_up_to_date():
    """입력은 build.py 로 결정적으로 재현되고 기대 결과는 현재 구현의 출력과 같다.
    동작을 일부러 바꿨다면 `python3 tests/golden/build.py` 로 다시 만들고 git diff 로 변경을 리뷰하라."""
    import build
    assert build.main(["--check"]) == 0


# 골든에서 관측되어야 하는 오류 코드. 나머지 둘은 골든으로 만들기 어렵다:
#   bad_encoding: utf-8/cp949 모두 실패하는 바이트열, participant_same: 같은 이름 두 번
#   (둘 다 단위 테스트에서 다룬다)
ERROR_CODES_IN_GOLDEN = {"empty_export", "mass_loss", "participant_not_found", "explicit_conversation_mismatch",
                         "room_title_ambiguous", "room_rename_ambiguous", "schema_too_old", "usage"}


def test_golden_covers_error_codes():
    seen = {c for scn in SCENARIOS for r in load_expected(scn)["results"] for c in r.get("error_codes", [])}
    assert seen == ERROR_CODES_IN_GOLDEN
