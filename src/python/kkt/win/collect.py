"""채팅방 하나의 대화를 내보내기(Ctrl+S)로 받아 파일로 저장한다.

흐름:
  창을 수집용 크기로 맞춤(끝나면 원래대로) -> 카카오톡 창을 앞으로 -> Ctrl+S
  -> 저장 대화상자를 찾아 파일명 칸에 경로를 넣고 저장 버튼 클릭 (포커스 없이 창 메시지로)
  -> 파일 생성 확인 -> 완료 팝업 닫기(Enter)

안전 규칙:
- 키는 카카오톡 창이 실제로 포커스일 때만 보낸다. 아니면 중단한다.
- Enter 는 완료 팝업 창이 포커스일 때만 보낸다 (입력창으로 가면 작성 중인 글이 전송될 수 있다).
- 이미 있는 파일은 덮어쓰지 않는다.
- Ctrl+D 로 언제든 중단. 중단/오류 시 열린 저장 대화상자를 닫고 창을 원래대로 돌린다.

Windows Python 으로 실행:
    python -m kkt.win.collect --title test --out <TXT를 저장할 폴더>
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

from . import winapi as api
from .guard import Aborted, Guard
from .window import (DEFAULT_HEIGHT, DEFAULT_WIDTH, WindowError, find_chat_windows,
                     normalized_window, set_dpi_aware)

_DEFAULT_NAME_RE = re.compile(r"^KakaoTalk_.+\.txt$")


class CollectError(Exception):
    pass


def _find(descs: list, cls: str, parent_cls: str | None = None, ctrl_id: int | None = None):
    for h, c, pc, cid in descs:
        if c == cls and (parent_cls is None or pc == parent_cls) and (ctrl_id is None or cid == ctrl_id):
            return h
    return None


def _new_popup(pid: int, known: set) -> int | None:
    """저장 후 새로 뜬 제목 없는 카카오톡 창(대화 내보내기 완료 팝업). 실측: 별도 최상위 창이다."""
    for h, cls, title in api.top_level_windows(pid):
        if cls == "EVA_Window_Dblclk" and title == "" and h not in known:
            return h
    return None


def export_chat(title: str, out_dir: Path, guard: Guard, width: int = DEFAULT_WIDTH,
                height: int = DEFAULT_HEIGHT, dialog_timeout: float = 8.0, save_timeout: float = 20.0,
                hold: float = 0.0) -> dict:
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    set_dpi_aware()
    dialog = None
    meta: dict = {"title": title, "warnings": []}
    try:
        with normalized_window(title, width, height) as info:
            meta["window"] = {"state_before": info["state_before"], "before": info["before"]}
            hwnd = info["hwnd"]
            pid = api.pid_of(hwnd)

            if api.top_level_dialogs(pid):
                raise CollectError("카카오톡에 이미 열린 대화상자가 있다. 닫은 뒤 다시 실행하라")
            known = {h for h, _, _ in api.top_level_windows(pid)}

            if not api.bring_to_front(hwnd, guard):
                raise CollectError("카카오톡 창을 앞으로 가져오지 못했다 (포커스를 확인하지 못해 키를 보내지 않는다)")
            guard.sleep(0.4)
            if api.foreground() != hwnd:
                raise CollectError("키를 보내기 직전에 포커스가 다른 창으로 옮겨졌다")
            api.tap(0x53, guard, mods=(api.VK_CONTROL,))                # Ctrl+S

            dialog = guard.wait_until(lambda: (api.top_level_dialogs(pid) or [None])[0], dialog_timeout)
            if not dialog:
                raise CollectError("저장 대화상자가 열리지 않았다")
            guard.sleep(0.3)
            if hold:                                                        # 디버그: Ctrl+D 중단 동작을 검증하는 용도
                guard.sleep(hold)
            descs = api.descendants(dialog)
            edit = _find(descs, "Edit", parent_cls="ComboBox")
            save_btn = _find(descs, "Button", ctrl_id=1)
            if edit is None or save_btn is None:
                raise CollectError("저장 대화상자에서 파일명 칸이나 저장 버튼을 찾지 못했다")
            default_name = api.text_of(edit).strip()
            if not _DEFAULT_NAME_RE.match(default_name):
                raise CollectError(f"기본 파일명이 예상과 다르다: {default_name!r}")
            target = out_dir / default_name
            if target.exists():
                raise CollectError(f"이미 있는 파일이다 (덮어쓰지 않음): {target}")
            api.set_text(edit, str(target))
            api.click_button(save_btn)

            def done():
                return target.exists() and target.stat().st_size > 0

            if not guard.wait_until(done, save_timeout):
                raise CollectError("내보낸 파일이 생성되지 않았다")
            size = target.stat().st_size
            guard.sleep(0.5)
            if target.stat().st_size != size:                              # 쓰는 중이면 한 번 더 기다린다
                guard.sleep(1.0)
            meta.update(path=str(target), size=target.stat().st_size, default_name=default_name)
            dialog = None

            # 완료 팝업은 별도 창이다. 포커스가 그 팝업일 때만 Enter 를 보낸다
            # (입력창으로 Enter 가 가면 작성 중인 글이 전송될 수 있다).
            popup = guard.wait_until(lambda: _new_popup(pid, known), 5.0)
            if not popup:
                meta["warnings"].append("완료 팝업을 찾지 못했다. 열려 있다면 직접 닫아야 한다")
            else:
                guard.sleep(0.3)
                if api.foreground() != popup and not api.bring_to_front(popup, guard):
                    meta["warnings"].append("완료 팝업에 포커스를 주지 못해 Enter 를 보내지 않았다")
                elif api.foreground() == popup:
                    api.tap(api.VK_RETURN, guard)
                    if not guard.wait_until(lambda: not api.is_visible(popup), 3.0):
                        meta["warnings"].append("Enter 를 보냈지만 완료 팝업이 닫히지 않았다")
            guard.sleep(0.3)
    finally:
        if dialog and api.is_window(dialog):
            api.close_window(dialog)                                        # 중단/오류 시 대화상자를 남기지 않는다
    meta["window"]["restore"] = info.get("restore")
    return meta


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="카카오톡 채팅방을 내보내기로 저장")
    ap.add_argument("--title", required=True)
    ap.add_argument("--out", required=True, help="TXT 를 저장할 폴더")
    ap.add_argument("--width", type=int, default=DEFAULT_WIDTH)
    ap.add_argument("--height", type=int, default=DEFAULT_HEIGHT)
    ap.add_argument("--hold", type=float, default=0.0, help="디버그: 저장 대화상자가 열린 뒤 N초 대기 (Ctrl+D 시험용)")
    ap.add_argument("--ingest", metavar="ARCHIVE", help="저장 직후 아카이브에 반영 (kkt ingest)")
    ap.add_argument("--conversation", help="--ingest 와 함께 쓰는 대화방 ID")
    a = ap.parse_args(argv)
    guard = Guard()
    try:
        meta = export_chat(a.title, Path(a.out), guard, a.width, a.height, hold=a.hold)
    except Aborted as e:
        print(f"[중단] {e}", file=sys.stderr)
        return 130
    except (CollectError, WindowError) as e:
        print(f"[실패] {e}", file=sys.stderr)
        return 1
    print(json.dumps(meta, ensure_ascii=False))
    if a.ingest:
        from ..cli import main as kkt_main
        args = ["--archive", a.ingest] + (["--conversation", a.conversation] if a.conversation else [])
        return kkt_main(args + ["ingest", meta["path"]])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
