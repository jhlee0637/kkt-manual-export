import pytest

from kkt.win.guard import Aborted, Guard
from kkt.win.window import Rect, plan_restore


class Clock:
    def __init__(self):
        self.t = 0.0

    def now(self):
        return self.t

    def sleep(self, s):
        self.t += s


def make_guard(pressed_at: float | None = None):
    c = Clock()
    g = Guard(abort_pressed=lambda: pressed_at is not None and c.t >= pressed_at,
              now=c.now, sleep=c.sleep)
    return g, c


def test_sleep_completes_without_abort():
    g, c = make_guard()
    g.sleep(1.0)
    assert c.t == pytest.approx(1.0)


def test_abort_is_detected_during_sleep():
    g, c = make_guard(pressed_at=0.5)
    with pytest.raises(Aborted):
        g.sleep(2.0)
    assert 0.5 <= c.t < 0.6                      # 대기 도중에 바로 감지된다 (끝까지 자지 않음)


def test_abort_is_sticky():
    g, _ = make_guard(pressed_at=0.0)
    with pytest.raises(Aborted):
        g.check()
    g._pressed = lambda: False                   # 키를 뗀 뒤에도 이미 중단된 상태
    with pytest.raises(Aborted):
        g.check()


def test_wait_until_returns_value_or_none_on_timeout():
    g, c = make_guard()
    calls = {"n": 0}

    def cond():
        calls["n"] += 1
        return "ok" if calls["n"] >= 3 else None

    assert g.wait_until(cond, timeout=5, step=0.1) == "ok"
    g2, _ = make_guard()
    assert g2.wait_until(lambda: None, timeout=0.5, step=0.1) is None


def test_wait_until_aborts():
    g, _ = make_guard(pressed_at=0.2)
    with pytest.raises(Aborted):
        g.wait_until(lambda: None, timeout=5, step=0.1)


B = Rect(459, 385, 1085, 450)


def test_restore_noop_when_already_original():
    assert plan_restore("normal", B, Rect(459, 385, 1085, 450)) == []
    assert plan_restore("normal", B, Rect(460, 384, 1086, 449)) == []        # 허용 오차 안


def test_restore_sets_original_rect():
    assert plan_restore("normal", B, Rect(459, 224, 380, 800)) == [("set_rect", B)]


def test_restore_minimized_and_maximized_states():
    assert plan_restore("minimized", B, Rect(459, 224, 380, 800)) == [("minimize", None)]
    assert plan_restore("maximized", B, Rect(459, 224, 380, 800)) == [("maximize", None)]

