"""카카오톡 채팅창을 수집하기 좋은 크기/위치로 맞춘다.

왜 필요한가: 수집은 화면 좌표(≡ 버튼, 서랍 선택 원 등)에 의존한다.
- 좌표는 창 기준 오른쪽/위 오프셋으로 계산하므로 창 크기가 바뀌어도 ≡ 는 유효하다 (실측: 380x640, 1085x450).
- 그러나 높이가 작으면 메시지 목록이 좁아져(450px 창에서 약 194px) 스크롤과 답장 이동 확인이 어렵다.
→ 수집 전에 창을 검증된 크기로 맞추고, 맞추지 못하면 중단한다.

사용 (Windows Python):
    python -m kkt.win.window --title test            # 맞춘다
    python -m kkt.win.window --title test --dry-run  # 계획만 출력
"""
from __future__ import annotations

import argparse
import json
import sys
from contextlib import contextmanager
from dataclasses import dataclass

CHAT_CLASS = "EVA_Window_Dblclk"
MAIN_WINDOW_TITLE = "카카오톡"

# 실측으로 ≡ 앵커가 동작한 크기: 380x640. 폭은 그대로 두고 높이는 가능하면 더 크게 쓴다.
DEFAULT_WIDTH = 380
DEFAULT_HEIGHT = 800
MIN_WIDTH = 380           # 이보다 좁으면 헤더 아이콘이 겹칠 수 있다 (미검증 → 거부)
MIN_HEIGHT = 560          # 이보다 낮으면 메시지 목록이 너무 좁다
SIZE_TOLERANCE = 2        # 카카오톡이 테두리 때문에 1~2px 다르게 적용할 수 있다


@dataclass(frozen=True)
class Rect:
    left: int
    top: int
    width: int
    height: int

    @property
    def right(self) -> int:
        return self.left + self.width

    @property
    def bottom(self) -> int:
        return self.top + self.height


class WindowError(Exception):
    pass


def plan_rect(current: Rect, work: Rect, width: int = DEFAULT_WIDTH, height: int = DEFAULT_HEIGHT,
              pos: tuple | None = None, margin: int = 8) -> Rect:
    """목표 사각형을 계산한다 (순수 함수).

    - 크기는 작업 영역(작업표시줄 제외)에 들어가도록 줄이되, 최소 크기보다 작아지면 오류.
    - 위치는 지정이 없으면 현재 왼쪽 위를 유지하고, 작업 영역을 벗어나면 안쪽으로 민다.
    """
    if width < MIN_WIDTH or height < MIN_HEIGHT:
        raise WindowError(f"요청 크기 {width}x{height}가 최소 {MIN_WIDTH}x{MIN_HEIGHT}보다 작다")
    w = min(width, work.width - 2 * margin)
    h = min(height, work.height - 2 * margin)
    if w < MIN_WIDTH or h < MIN_HEIGHT:
        raise WindowError(
            f"작업 영역 {work.width}x{work.height}에 {MIN_WIDTH}x{MIN_HEIGHT} 이상의 창을 둘 수 없다")
    left, top = pos if pos else (current.left, current.top)
    left = max(work.left + margin, min(left, work.right - margin - w))
    top = max(work.top + margin, min(top, work.bottom - margin - h))
    return Rect(left, top, w, h)


def within_tolerance(actual: Rect, target: Rect, tol: int = SIZE_TOLERANCE) -> bool:
    return (abs(actual.width - target.width) <= tol and abs(actual.height - target.height) <= tol
            and abs(actual.left - target.left) <= tol and abs(actual.top - target.top) <= tol)


def plan_restore(state_before: str, rect_before: Rect, rect_now: Rect) -> list:
    """수집 후 창을 원래대로 돌리는 동작 목록 (순수 함수).

    최소화/최대화였던 창은 그 상태로 되돌린다 (normalize 가 먼저 일반 상태로 복원해서 크기를 쟀다).
    일반 상태였던 창은 원래 사각형으로 되돌리되, 이미 같으면 아무것도 하지 않는다.
    """
    if state_before == "minimized":
        return [("minimize", None)]
    if state_before == "maximized":
        return [("maximize", None)]
    if within_tolerance(rect_now, rect_before):
        return []
    return [("set_rect", rect_before)]


# ── Windows API ──────────────────────────────────────────
def _win():
    if not sys.platform.startswith("win"):
        raise WindowError("Windows 에서만 동작한다 (Windows Python 으로 실행하라)")
    import ctypes
    from ctypes import wintypes
    return ctypes, wintypes


def set_dpi_aware() -> None:
    """좌표가 물리 픽셀 기준이 되도록. 배율이 100%가 아닐 때 클릭이 빗나가는 것을 막는다."""
    ctypes, _ = _win()
    try:
        if ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4)):
            return
    except Exception:
        pass
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
    except Exception:
        pass


def find_chat_windows(title: str) -> list:
    """제목이 정확히 일치하는 채팅창 핸들 목록. 메인 창(카카오톡)은 제외한다."""
    ctypes, wintypes = _win()
    u = ctypes.windll.user32
    if title == MAIN_WINDOW_TITLE:
        raise WindowError("메인 창은 대상이 아니다")
    found: list = []
    cb_t = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def cb(hwnd, _):
        if not u.IsWindowVisible(hwnd):
            return True
        cls = ctypes.create_unicode_buffer(256)
        u.GetClassNameW(hwnd, cls, 256)
        if cls.value != CHAT_CLASS:
            return True
        n = u.GetWindowTextLengthW(hwnd)
        buf = ctypes.create_unicode_buffer(n + 1)
        u.GetWindowTextW(hwnd, buf, n + 1)
        if buf.value == title:
            found.append(int(hwnd))
        return True

    u.EnumWindows(cb_t(cb), 0)
    return found


def get_rect(hwnd: int) -> Rect:
    ctypes, wintypes = _win()
    r = wintypes.RECT()
    if not ctypes.windll.user32.GetWindowRect(wintypes.HWND(hwnd), ctypes.byref(r)):
        raise WindowError("GetWindowRect 실패")
    return Rect(r.left, r.top, r.right - r.left, r.bottom - r.top)


def work_area_of(hwnd: int) -> Rect:
    """창이 놓인 모니터의 작업 영역(작업표시줄 제외)."""
    ctypes, wintypes = _win()

    class MONITORINFO(ctypes.Structure):
        _fields_ = [("cbSize", wintypes.DWORD), ("rcMonitor", wintypes.RECT),
                    ("rcWork", wintypes.RECT), ("dwFlags", wintypes.DWORD)]

    u = ctypes.windll.user32
    u.MonitorFromWindow.restype = wintypes.HANDLE
    mon = u.MonitorFromWindow(wintypes.HWND(hwnd), 2)      # MONITOR_DEFAULTTONEAREST
    mi = MONITORINFO()
    mi.cbSize = ctypes.sizeof(MONITORINFO)
    if not u.GetMonitorInfoW(mon, ctypes.byref(mi)):
        raise WindowError("GetMonitorInfo 실패")
    w = mi.rcWork
    return Rect(w.left, w.top, w.right - w.left, w.bottom - w.top)


def _state(hwnd: int) -> str:
    ctypes, wintypes = _win()
    u = ctypes.windll.user32
    if u.IsIconic(wintypes.HWND(hwnd)):
        return "minimized"
    if u.IsZoomed(wintypes.HWND(hwnd)):
        return "maximized"
    return "normal"


def normalize(title: str, width: int = DEFAULT_WIDTH, height: int = DEFAULT_HEIGHT,
              pos: tuple | None = None, dry_run: bool = False) -> dict:
    set_dpi_aware()
    ctypes, wintypes = _win()
    u = ctypes.windll.user32
    hs = find_chat_windows(title)
    if not hs:
        raise WindowError(f"제목이 {title!r}인 채팅창이 없다 (창으로 열려 있어야 한다)")
    if len(hs) > 1:
        raise WindowError(f"제목이 {title!r}인 창이 {len(hs)}개다. 대상을 특정할 수 없어 중단한다")
    hwnd = hs[0]
    state = _state(hwnd)
    if state != "normal" and not dry_run:
        u.ShowWindow(wintypes.HWND(hwnd), 9)               # SW_RESTORE
    before = get_rect(hwnd)
    target = plan_rect(before, work_area_of(hwnd), width, height, pos)
    result = {"hwnd": hwnd, "state_before": state, "before": before.__dict__ | {"right": before.right},
              "target": target.__dict__, "changed": False}
    if dry_run:
        return result
    if not within_tolerance(before, target):
        SWP_NOZORDER, SWP_NOACTIVATE = 0x0004, 0x0010      # 포커스를 가져가지 않는다
        if not u.SetWindowPos(wintypes.HWND(hwnd), None, target.left, target.top, target.width,
                              target.height, SWP_NOZORDER | SWP_NOACTIVATE):
            raise WindowError("SetWindowPos 실패")
        result["changed"] = True
    after = get_rect(hwnd)
    result["after"] = after.__dict__
    if not within_tolerance(after, target):
        raise WindowError(
            f"창 크기를 맞추지 못했다: 목표 {target.width}x{target.height}, 실제 {after.width}x{after.height}")
    return result


def restore(title: str, info: dict) -> dict:
    """normalize 이전의 크기/위치/상태로 되돌린다. 실패해도 예외를 던지지 않고 결과에 남긴다
    (중단·오류 처리 중에 호출되므로 원래 예외를 가리면 안 된다)."""
    out = {"restored": False, "actions": []}
    try:
        set_dpi_aware()
        ctypes, wintypes = _win()
        u = ctypes.windll.user32
        hs = find_chat_windows(title)
        if len(hs) != 1:
            out["warning"] = f"복원할 창을 특정하지 못했다 (일치 {len(hs)}개)"
            return out
        hwnd = hs[0]
        b = info["before"]
        before = Rect(b["left"], b["top"], b["width"], b["height"])
        for act, arg in plan_restore(info["state_before"], before, get_rect(hwnd)):
            out["actions"].append(act)
            if act == "minimize":
                u.ShowWindow(wintypes.HWND(hwnd), 6)           # SW_MINIMIZE
            elif act == "maximize":
                u.ShowWindow(wintypes.HWND(hwnd), 3)           # SW_MAXIMIZE
            else:
                u.SetWindowPos(wintypes.HWND(hwnd), None, arg.left, arg.top, arg.width, arg.height,
                               0x0004 | 0x0010)
        if info["state_before"] == "normal":
            out["restored"] = within_tolerance(get_rect(hwnd), before)
        else:
            out["restored"] = True
    except Exception as e:                                   # noqa: BLE001
        out["warning"] = f"복원 중 오류: {e}"
    return out


@contextmanager
def normalized_window(title: str, width: int = DEFAULT_WIDTH, height: int = DEFAULT_HEIGHT,
                      pos: tuple | None = None):
    """with 블록 동안 창을 수집용으로 맞추고, 블록이 끝나면(오류/중단 포함) 원래대로 돌린다."""
    info = normalize(title, width, height, pos)
    result: dict = {}
    try:
        yield info
    finally:
        result.update(restore(title, info))
        info["restore"] = result


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="카카오톡 채팅창을 수집용 크기로 맞춘다")
    ap.add_argument("--title", required=True, help="채팅창 제목 (정확히 일치)")
    ap.add_argument("--width", type=int, default=DEFAULT_WIDTH)
    ap.add_argument("--height", type=int, default=DEFAULT_HEIGHT)
    ap.add_argument("--pos", help="왼쪽,위 (기본: 현재 위치 유지)")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args(argv)
    pos = tuple(int(v) for v in a.pos.split(",")) if a.pos else None
    try:
        print(json.dumps(normalize(a.title, a.width, a.height, pos, a.dry_run), ensure_ascii=False))
    except WindowError as e:
        print(f"[중단] {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
