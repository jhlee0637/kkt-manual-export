"""긴급 탈출 (Ctrl+D).

전역 단축키로 등록하지 않는다. 등록하면 봇이 도는 동안 모든 프로그램의 Ctrl+D 가 가로채진다.
대신 GetAsyncKeyState 로 폴링해서 키를 소비하지 않고 감지한다.
봇은 VK_D 를 주입하지 않는다 (파일 경로는 유니코드 메시지로 넣는다). 그래서 봇 자신의 입력과 섞이지 않는다.
모든 대기는 Guard.sleep 을 거치므로 대기 중에도 감지된다.
"""
from __future__ import annotations

import sys
import time
from typing import Callable

VK_CONTROL = 0x11
VK_D = 0x44


class Aborted(Exception):
    """사용자가 Ctrl+D 로 중단했다."""


def _default_key_state() -> bool:
    if not sys.platform.startswith("win"):
        return False
    import ctypes
    gs = ctypes.windll.user32.GetAsyncKeyState
    return bool(gs(VK_CONTROL) & 0x8000) and bool(gs(VK_D) & 0x8000)


class Guard:
    def __init__(self, abort_pressed: Callable[[], bool] | None = None,
                 now: Callable[[], float] = time.monotonic, sleep: Callable[[float], None] = time.sleep):
        self._pressed = abort_pressed or _default_key_state
        self._now = now
        self._sleep = sleep
        self.aborted = False

    def check(self) -> None:
        if self.aborted or self._pressed():
            self.aborted = True
            raise Aborted("Ctrl+D 로 중단됨")

    def sleep(self, seconds: float, step: float = 0.03) -> None:
        end = self._now() + seconds
        while True:
            self.check()
            left = end - self._now()
            if left <= 0:
                return
            self._sleep(min(step, left))

    def wait_until(self, cond: Callable[[], object], timeout: float, step: float = 0.1):
        """cond()가 참 같은 값을 돌려줄 때까지 기다린다. 시간 초과면 None."""
        end = self._now() + timeout
        while True:
            self.check()
            v = cond()
            if v:
                return v
            if self._now() >= end:
                return None
            self._sleep(step)
