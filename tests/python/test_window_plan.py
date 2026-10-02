import pytest

from kkt.win.window import Rect, WindowError, plan_rect, within_tolerance

WORK = Rect(0, 0, 1920, 1040)      # 1080p 에서 작업표시줄을 뺀 영역


def test_keeps_position_when_it_fits():
    t = plan_rect(Rect(459, 385, 1085, 450), WORK, 380, 800)
    assert (t.width, t.height) == (380, 800)
    # 왼쪽은 그대로, 아래로 넘치므로 위쪽만 1040-8-800=232 로 밀린다
    assert (t.left, t.top) == (459, 232)


def test_pushes_inside_work_area():
    t = plan_rect(Rect(1800, 900, 380, 640), WORK, 380, 640)
    assert t.right <= WORK.right - 8 and t.bottom <= WORK.bottom - 8
    t = plan_rect(Rect(-300, -50, 380, 640), WORK, 380, 640)
    assert t.left >= 8 and t.top >= 8


def test_shrinks_height_to_work_area():
    t = plan_rect(Rect(0, 0, 380, 640), Rect(0, 0, 1366, 728), 380, 800)
    assert t.height == 728 - 16 and t.width == 380


def test_explicit_position():
    t = plan_rect(Rect(0, 0, 380, 640), WORK, 380, 640, pos=(100, 50))
    assert (t.left, t.top) == (100, 50)


def test_rejects_too_small_request_and_too_small_screen():
    with pytest.raises(WindowError):
        plan_rect(Rect(0, 0, 380, 640), WORK, 300, 640)
    with pytest.raises(WindowError):
        plan_rect(Rect(0, 0, 380, 640), WORK, 380, 400)
    with pytest.raises(WindowError):
        plan_rect(Rect(0, 0, 380, 640), Rect(0, 0, 800, 500), 380, 800)   # 작업 영역 높이 부족


def test_within_tolerance():
    assert within_tolerance(Rect(10, 10, 381, 799), Rect(10, 10, 380, 800))
    assert not within_tolerance(Rect(10, 10, 390, 800), Rect(10, 10, 380, 800))
