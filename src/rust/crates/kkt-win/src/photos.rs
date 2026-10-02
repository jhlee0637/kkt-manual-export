//! 채팅방 서랍의 사진을 파일로 저장한다.
//!
//! 흐름: 채팅창을 수집용 크기로 맞춤 → `≡` 메뉴 → `채팅방 서랍` → `사진/동영상` → 서랍 창에서 최신 사진부터
//! 선택 원을 눌러 고름 → 선택 바의 다운로드 아이콘 → 저장 결과 팝업을 `Esc` 로 닫음 → 저장 폴더의 새 파일 확인.
//!
//! 안전 규칙 (수집과 같다):
//! - 키와 클릭은 대상 창이 포커스일 때만 보낸다. 아니면 중단한다.
//! - 서랍 창 왼쪽에는 이 계정의 모든 방 목록이 있다. 그쪽은 건드리지 않는다 (오른쪽 격자와 아래 선택 바만 누른다).
//! - 선택 바의 왼쪽 아이콘은 "전달"이다. 누르지 않는다. 다운로드 아이콘은 화면 모양을 확인한 뒤에만 누른다.
//! - 저장 결과 팝업은 `Esc` 로 닫는다 (`Enter` 는 `폴더 열기` 를 누를 수 있다).
//! - Ctrl+D 로 언제든 중단. 중단/오류 시 열린 메뉴와 서랍 창을 닫고 채팅창을 원래대로 돌린다.

use crate::grid::{self, Cell};
use crate::guard::Guard;
use crate::sys::{self, Hwnd};
use crate::window::{self, CHAT_CLASS, DRAWER_TITLE};
use crate::WinError;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Options {
    /// 저장할 최신 사진 수. `None` 이면 전부.
    pub newest: Option<usize>,
    pub width: i32,
    pub height: i32,
}

impl Default for Options {
    fn default() -> Self {
        Options { newest: None, width: window::DEFAULT_WIDTH, height: window::DEFAULT_HEIGHT }
    }
}

fn fail<T>(msg: impl Into<String>) -> Result<T, WinError> {
    Err(WinError::Collect(msg.into()))
}

/// 카카오톡의 기본 저장 폴더 `문서\카카오톡 받은 파일`.
pub fn default_save_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    home.join("Documents").join("카카오톡 받은 파일")
}

fn listing(dir: &Path) -> HashMap<String, u64> {
    let mut m = HashMap::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    m.insert(e.file_name().to_string_lossy().to_string(), md.len());
                }
            }
        }
    }
    m
}

/// 대상 창을 앞으로 가져오고 포커스를 확인한다.
fn front(h: Hwnd, guard: &Guard, what: &str) -> Result<(), WinError> {
    if !sys::bring_to_front(h, guard, 2.0)? {
        return fail(format!("{what}을(를) 앞으로 가져오지 못했다 (포커스를 확인하지 못해 입력을 보내지 않는다)"));
    }
    Ok(())
}

fn require_focus(h: Hwnd, what: &str) -> Result<(), WinError> {
    if sys::foreground() != h {
        return fail(format!("입력을 보내기 직전에 포커스가 {what} 밖으로 옮겨졌다"));
    }
    Ok(())
}

fn key(vk: u16, guard: &Guard) -> Result<(), WinError> {
    sys::tap(vk, guard, &[])
}

fn click_at(x: i32, y: i32, guard: &Guard) -> Result<(), WinError> {
    guard.check()?;
    sys::cursor_to(x, y);
    guard.sleep(0.12)?;
    sys::mouse_click();
    Ok(())
}

fn hover(x: i32, y: i32, guard: &Guard) -> Result<(), WinError> {
    sys::cursor_to(x - 3, y - 2);
    guard.sleep(0.1)?;
    sys::cursor_to(x, y);
    guard.sleep(0.2)
}

/// `≡` → `채팅방 서랍` → `사진/동영상`. 서랍 창 핸들을 돌려준다.
fn open_drawer(chat: Hwnd, guard: &Guard) -> Result<Hwnd, WinError> {
    front(chat, guard, "카카오톡 채팅창")?;
    guard.sleep(0.4)?;
    require_focus(chat, "채팅창")?;
    let r = sys::window_rect(chat).ok_or_else(|| WinError::Window("GetWindowRect 실패".into()))?;
    let (ex, ey) = (r.right - 22, r.top + 57); // ≡ (실측: 창 폭 −22, 위에서 57)
    click_at(ex, ey, guard)?;
    guard.sleep(0.7)?;
    require_focus(chat, "채팅창")?;
    hover(ex - 42, ey + 93, guard)?; // `채팅방 서랍` (실측: ≡ 에서 아래 93, 왼쪽 42)
    guard.sleep(0.3)?;
    key(sys::VK_RIGHT, guard)?; // 하위 메뉴는 클릭이 아니라 오른쪽 방향키로 연다
    guard.sleep(0.6)?;
    require_focus(chat, "채팅창")?;
    key(sys::VK_DOWN, guard)?; // 하위 메뉴: 사진/동영상, 파일, 링크 (모두 읽기 항목)
    guard.sleep(0.3)?;
    key(sys::VK_RETURN, guard)?;
    let drawer = guard.wait_until(|| sys::find_top_level(CHAT_CLASS, DRAWER_TITLE).first().copied(), 5.0, 0.2)?;
    match drawer {
        Some(d) => Ok(d),
        None => {
            // 메뉴가 남아 있을 수 있다
            if sys::foreground() == chat {
                let _ = key(sys::VK_ESCAPE, guard);
                let _ = key(sys::VK_ESCAPE, guard);
            }
            fail("채팅방 서랍이 열리지 않았다 (메뉴 위치가 다를 수 있다)")
        }
    }
}

struct Selection {
    selected: usize,
    seen: usize,
}

/// 서랍 격자에서 최신 사진부터 `limit` 장을 고른다.
fn select_photos(drawer: Hwnd, limit: usize, guard: &Guard) -> Result<Selection, WinError> {
    let mut seen: HashSet<u64> = HashSet::new();
    let mut selected = 0usize;
    let mut stall = 0;
    for _round in 0..400 {
        guard.check()?;
        let r = sys::window_rect(drawer).ok_or_else(|| WinError::Window("GetWindowRect 실패".into()))?;
        let Some(img) = sys::capture_window(drawer) else { return fail("서랍 창을 캡처하지 못했다") };
        let cells = grid::find_cells(&img);
        let fresh: Vec<&Cell> = cells.iter().filter(|c| !seen.contains(&c.sig)).collect();
        if fresh.is_empty() {
            stall += 1;
        } else {
            stall = 0;
        }
        for c in fresh {
            if selected >= limit {
                break;
            }
            seen.insert(c.sig);
            if !c.selected {
                front(drawer, guard, "서랍 창")?;
                require_focus(drawer, "서랍 창")?;
                let (cx, cy) = c.circle();
                // 썸네일 위로 먼저 올려서 선택 원을 띄운 뒤 누른다
                hover(r.left + c.x + 62, r.top + c.y + 62, guard)?;
                click_at(r.left + cx, r.top + cy, guard)?;
                guard.sleep(0.25)?;
            }
            selected += 1;
        }
        if selected >= limit || stall >= 2 {
            break;
        }
        // 아래로 스크롤 (격자 위에서)
        front(drawer, guard, "서랍 창")?;
        sys::cursor_to(r.left + grid::GRID_LEFT + 150, r.top + 380);
        guard.sleep(0.1)?;
        sys::wheel(-3);
        guard.sleep(0.5)?;
    }
    Ok(Selection { selected, seen: seen.len() })
}

/// 사진을 저장한다. `save_dir` 는 카카오톡이 저장하는 폴더 (기본: `문서\카카오톡 받은 파일`).
pub fn download_photos(title: &str, save_dir: &Path, guard: &Guard, opt: &Options) -> Result<Value, WinError> {
    if opt.newest == Some(0) {
        return Ok(json!({"title": title, "selected": 0, "saved_files": [], "warnings": []}));
    }
    sys::set_dpi_aware();
    let info = window::normalize(title, opt.width, opt.height, None)?;
    let mut meta = json!({"title": title, "warnings": [], "window": info.summary()});
    let mut drawer: Option<Hwnd> = None;
    let res = run(save_dir, guard, opt, &info, &mut meta, &mut drawer);
    if let Some(d) = drawer {
        if sys::is_window(d) {
            sys::close_window(d); // 서랍 창을 남기지 않는다
        }
    }
    let restore = window::restore(title, &info);
    res?;
    meta["window"]["restore"] = restore;
    Ok(meta)
}

fn warn(meta: &mut Value, msg: &str) {
    meta["warnings"].as_array_mut().expect("warnings 는 배열").push(Value::from(msg));
}

fn run(save_dir: &Path, guard: &Guard, opt: &Options, info: &window::Info, meta: &mut Value, drawer_out: &mut Option<Hwnd>) -> Result<(), WinError> {
    let chat = info.hwnd;
    let pid = sys::pid_of(chat);
    let before = listing(save_dir);
    let drawer = open_drawer(chat, guard)?;
    *drawer_out = Some(drawer);
    front(drawer, guard, "서랍 창")?;
    guard.sleep(0.6)?;

    let sel = select_photos(drawer, opt.newest.unwrap_or(usize::MAX), guard)?;
    meta["selected"] = Value::from(sel.selected);
    if sel.selected == 0 {
        return fail("서랍에서 사진을 찾지 못했다 (사진이 없거나 화면 구성이 다르다)");
    }
    if let Some(n) = opt.newest {
        if sel.selected < n {
            warn(meta, &format!("최신 {n}장을 요청했지만 서랍에서 {}장만 찾았다", sel.selected));
        }
    }

    // 다운로드 아이콘: 선택 바의 모양을 확인한 뒤에만 누른다
    front(drawer, guard, "서랍 창")?;
    guard.sleep(0.4)?;
    let Some(img) = sys::capture_window(drawer) else { return fail("서랍 창을 캡처하지 못했다") };
    if !grid::selection_bar_visible(&img) {
        return fail("선택 바의 모양이 예상과 달라 다운로드를 누르지 않았다");
    }
    let known: HashSet<Hwnd> = sys::top_level_windows(pid).into_iter().map(|d| d.0).collect();
    let r = sys::window_rect(drawer).ok_or_else(|| WinError::Window("GetWindowRect 실패".into()))?;
    let (dx, dy) = grid::download_icon(&img);
    require_focus(drawer, "서랍 창")?;
    click_at(r.left + dx, r.top + dy, guard)?;

    // 저장 결과 팝업 (별도 창, 제목 없음). `Esc` 로 닫는다.
    let popup = guard.wait_until(
        || {
            sys::top_level_windows(pid)
                .into_iter()
                .find(|(h, cls, t)| cls == CHAT_CLASS && t.is_empty() && !known.contains(h))
                .map(|d| d.0)
        },
        8.0,
        0.2,
    )?;
    match popup {
        None => warn(meta, "저장 결과 팝업을 찾지 못했다. 열려 있다면 직접 닫아야 한다"),
        Some(p) => {
            guard.sleep(0.3)?;
            if sys::foreground() != p && !sys::bring_to_front(p, guard, 2.0)? {
                warn(meta, "저장 결과 팝업에 포커스를 주지 못해 닫지 않았다");
            } else if sys::foreground() == p {
                key(sys::VK_ESCAPE, guard)?;
                if guard.wait_until(|| (!sys::is_visible(p)).then_some(()), 3.0, 0.1)?.is_none() {
                    warn(meta, "Esc 를 보냈지만 저장 결과 팝업이 닫히지 않았다");
                }
            }
        }
    }

    // 저장 폴더의 새 파일로 확인한다
    let expected = sel.selected;
    let new_files = |dir: &Path| -> Vec<String> {
        let now = listing(dir);
        let mut v: Vec<String> = now.keys().filter(|k| !before.contains_key(*k)).cloned().collect();
        v.sort();
        v
    };
    let ok = guard.wait_until(|| (new_files(save_dir).len() >= expected).then_some(()), 20.0, 0.3)?;
    let files = new_files(save_dir);
    meta["saved_files"] = json!(files);
    meta["seen_cells"] = Value::from(sel.seen);
    if ok.is_none() {
        warn(meta, &format!("저장된 새 파일이 {}개뿐이다 (선택 {expected}개). 저장 폴더가 기본 위치가 아니거나 이미 있는 이름일 수 있다", files.len()));
    }
    Ok(())
}
