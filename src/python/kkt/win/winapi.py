"""ctypes 로 감싼 Win32 도우미. 안전 규칙: 키는 대상 창이 포커스일 때만 보낸다."""
from __future__ import annotations

import sys
import time

from .guard import Guard


def _w():
    if not sys.platform.startswith("win"):
        raise OSError("Windows 에서만 동작한다")
    import ctypes
    from ctypes import wintypes
    return ctypes, wintypes


WM_SETTEXT, WM_GETTEXT, WM_GETTEXTLENGTH, WM_CLOSE, BM_CLICK = 0x000C, 0x000D, 0x000E, 0x0010, 0x00F5
VK_CONTROL, VK_RETURN, VK_ESCAPE, VK_MENU = 0x11, 0x0D, 0x1B, 0x12


def _user32():
    ctypes, wintypes = _w()
    u = ctypes.windll.user32
    u.SendMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, ctypes.c_void_p]
    u.SendMessageW.restype = ctypes.c_ssize_t
    u.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    u.GetForegroundWindow.restype = wintypes.HWND
    u.SetForegroundWindow.argtypes = [wintypes.HWND]
    u.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    u.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    u.GetParent.argtypes = [wintypes.HWND]
    u.GetParent.restype = wintypes.HWND
    u.GetDlgCtrlID.argtypes = [wintypes.HWND]
    u.IsWindow.argtypes = [wintypes.HWND]
    u.IsWindowVisible.argtypes = [wintypes.HWND]
    return u


def pid_of(hwnd: int) -> int:
    ctypes, wintypes = _w()
    pid = wintypes.DWORD()
    _user32().GetWindowThreadProcessId(wintypes.HWND(hwnd), ctypes.byref(pid))
    return int(pid.value)


def class_of(hwnd: int) -> str:
    ctypes, wintypes = _w()
    buf = ctypes.create_unicode_buffer(256)
    _user32().GetClassNameW(wintypes.HWND(hwnd), buf, 256)
    return buf.value


def text_of(hwnd: int) -> str:
    ctypes, wintypes = _w()
    u = _user32()
    n = u.SendMessageW(wintypes.HWND(hwnd), WM_GETTEXTLENGTH, 0, None)
    buf = ctypes.create_unicode_buffer(int(n) + 1)
    u.SendMessageW(wintypes.HWND(hwnd), WM_GETTEXT, int(n) + 1, ctypes.cast(buf, ctypes.c_void_p))
    return buf.value


def set_text(hwnd: int, value: str) -> None:
    ctypes, wintypes = _w()
    buf = ctypes.create_unicode_buffer(value)
    _user32().SendMessageW(wintypes.HWND(hwnd), WM_SETTEXT, 0, ctypes.cast(buf, ctypes.c_void_p))


def click_button(hwnd: int) -> None:
    ctypes, wintypes = _w()
    _user32().SendMessageW(wintypes.HWND(hwnd), BM_CLICK, 0, None)


def close_window(hwnd: int) -> None:
    ctypes, wintypes = _w()
    _user32().PostMessageW(wintypes.HWND(hwnd), WM_CLOSE, 0, 0)


def is_window(hwnd: int) -> bool:
    ctypes, wintypes = _w()
    return bool(_user32().IsWindow(wintypes.HWND(hwnd)))


def foreground() -> int:
    ctypes, wintypes = _w()
    return int(_user32().GetForegroundWindow() or 0)


def descendants(hwnd: int) -> list:
    """[(hwnd, class, parent_class, ctrl_id)] 모든 하위 창."""
    ctypes, wintypes = _w()
    u = _user32()
    out: list = []
    cb_t = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def cb(h, _):
        p = u.GetParent(h)
        out.append((int(h), class_of(int(h)), class_of(int(p)) if p else "", int(u.GetDlgCtrlID(h))))
        return True

    u.EnumChildWindows.argtypes = [wintypes.HWND, cb_t, wintypes.LPARAM]
    u.EnumChildWindows(wintypes.HWND(hwnd), cb_t(cb), 0)
    return out


def top_level_dialogs(pid: int) -> list:
    """해당 프로세스의 보이는 #32770(공용 대화상자) 최상위 창."""
    ctypes, wintypes = _w()
    u = _user32()
    out: list = []
    cb_t = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def cb(h, _):
        if u.IsWindowVisible(h) and class_of(int(h)) == "#32770" and pid_of(int(h)) == pid:
            out.append(int(h))
        return True

    u.EnumWindows.argtypes = [cb_t, wintypes.LPARAM]
    u.EnumWindows(cb_t(cb), 0)
    return out


def top_level_windows(pid: int) -> list:
    """해당 프로세스의 보이는 최상위 창 [(hwnd, class, title)]. 완료 팝업처럼 제목 없는 창도 포함한다."""
    ctypes, wintypes = _w()
    u = _user32()
    out: list = []
    cb_t = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def cb(h, _):
        if u.IsWindowVisible(h) and pid_of(int(h)) == pid:
            out.append((int(h), class_of(int(h)), text_of(int(h))))
        return True

    u.EnumWindows.argtypes = [cb_t, wintypes.LPARAM]
    u.EnumWindows(cb_t(cb), 0)
    return out


def is_visible(hwnd: int) -> bool:
    ctypes, wintypes = _w()
    u = _user32()
    return bool(u.IsWindow(wintypes.HWND(hwnd)) and u.IsWindowVisible(wintypes.HWND(hwnd)))


# ── 입력 (SendInput) ─────────────────────────────────────
def _send_keys(events: list) -> None:
    """events: [(vk, down)]. 유니코드 문자는 쓰지 않는다 (VK_D 를 주입하지 않기 위해 경로는 메시지로 넣는다)."""
    ctypes, wintypes = _w()

    class KEYBDINPUT(ctypes.Structure):
        _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD),
                    ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_void_p)]

    class MOUSEINPUT(ctypes.Structure):
        _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
                    ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_void_p)]

    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT)]

    class INPUT(ctypes.Structure):
        _fields_ = [("type", wintypes.DWORD), ("u", _U)]

    arr = (INPUT * len(events))()
    for i, (vk, down) in enumerate(events):
        arr[i].type = 1
        arr[i].u.ki = KEYBDINPUT(vk, 0, 0 if down else 2, 0, None)
    n = ctypes.windll.user32.SendInput(len(events), arr, ctypes.sizeof(INPUT))
    if n != len(events):
        raise OSError(f"SendInput 이 {n}/{len(events)}개만 주입했다 (권한이 높은 창이 포커스일 수 있다)")


def tap(vk: int, guard: Guard, mods: tuple = ()) -> None:
    """수정키를 확실히 떼도록 try/finally 로 보낸다."""
    guard.check()
    downs = [(m, True) for m in mods] + [(vk, True)]
    ups = [(vk, False)] + [(m, False) for m in reversed(mods)]
    try:
        _send_keys(downs)
        time.sleep(0.04)
    finally:
        _send_keys(ups)


def bring_to_front(hwnd: int, guard: Guard, timeout: float = 2.0) -> bool:
    """대상 창을 앞으로 가져오고 실제로 포커스가 갔는지 확인한다. 실패하면 False."""
    ctypes, wintypes = _w()
    u = _user32()
    if foreground() == hwnd:
        return True
    _send_keys([(VK_MENU, True), (VK_MENU, False)])        # 포그라운드 전환 제한을 푸는 관용 기법
    u.SetForegroundWindow(wintypes.HWND(hwnd))
    return bool(guard.wait_until(lambda: foreground() == hwnd, timeout, 0.05))
