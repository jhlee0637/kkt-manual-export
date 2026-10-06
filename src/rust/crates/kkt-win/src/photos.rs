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
use crate::guard::{Guard, PauseMode};
use crate::sys::{self, Hwnd};
use crate::window::{self, CHAT_CLASS, DRAWER_TITLE};
use crate::WinError;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Options {
    /// 고를 최신 **타일** 수. `None` 이면 전부. 서랍의 타일은 사진이 아니라 메시지 하나다
    /// (`사진 16장` 한 번에 보낸 묶음은 타일 하나이고, 저장하면 파일 16개가 된다).
    pub newest: Option<usize>,
    /// 저장될 것으로 기대하는 파일 수 (묶음은 장수의 합). `None` 이면 타일 수.
    pub expect_files: Option<usize>,
    /// 동영상 타일은 고르지 않는다 (동영상을 보관하지 않을 때).
    pub skip_videos: bool,
    /// 아무것도 누르지 않고 서랍을 끝까지 훑어 타일 수만 센다 (진단용).
    pub dry_run: bool,
    pub width: i32,
    pub height: i32,
}

impl Default for Options {
    fn default() -> Self {
        Options { newest: None, expect_files: None, skip_videos: false, dry_run: false, width: window::DEFAULT_WIDTH, height: window::DEFAULT_HEIGHT }
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

/// 현장 진단용: 환경변수 `KKT_DEBUG_DIR` 가 있으면 캡처를 BMP 로 저장한다 (없으면 아무것도 하지 않는다).
fn dump(img: &sys::Image, name: &str) {
    let Some(dir) = std::env::var_os("KKT_DEBUG_DIR") else { return };
    let (w, h) = (img.w, img.h);
    let row = (w * 3 + 3) & !3;
    let mut out = Vec::with_capacity(54 + row * h);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + row * h) as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes()); // 위에서 아래로
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            out.extend_from_slice(&img.px[i..i + 3]); // B, G, R
        }
        out.extend(std::iter::repeat(0u8).take(row - w * 3));
    }
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(Path::new(&dir).join(format!("{name}.bmp")), out);
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

/// 마우스 때문에 일시정지했다가 이어가면(`Interrupted`) 단계를 처음부터 다시 한다. `attempt` 는 0부터.
/// 사용자가 클릭했을 수도 있어서 화면 상태를 장담할 수 없으므로, 단계는 시작할 때 상태를 다시 확인해야 한다.
const MAX_RESUMES: u32 = 20;

fn with_resume<T>(mut step: impl FnMut(u32) -> Result<T, WinError>) -> Result<T, WinError> {
    for attempt in 0..=MAX_RESUMES {
        match step(attempt) {
            Err(WinError::Interrupted) => continue,
            other => return other,
        }
    }
    fail("마우스를 너무 자주 움직여서 작업을 이어가지 못했다")
}

/// 클릭한다. 마우스를 옮기기 전과 누르기 직전에 확인하므로, 사용자가 움직였으면 누르지 않고 `Interrupted` 가 된다.
fn click_at(x: i32, y: i32, guard: &Guard) -> Result<(), WinError> {
    guard.check()?;
    guard.move_cursor(x, y);
    guard.sleep(0.12)?;
    sys::mouse_click();
    Ok(())
}

fn hover(x: i32, y: i32, guard: &Guard) -> Result<(), WinError> {
    guard.move_cursor(x - 3, y - 2);
    guard.sleep(0.1)?;
    guard.move_cursor(x, y);
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
    videos: usize,
}

#[derive(Default)]
struct SelState {
    seen: HashSet<u64>,
    selected: usize,
    videos: usize,
    stall: u32,
}

/// 서랍 격자의 한 화면을 읽어 새 타일을 고르고 스크롤한다. 끝났으면 `Ok(true)`.
fn select_round(drawer: Hwnd, limit: usize, opt: &Options, guard: &Guard, st: &mut SelState, round: usize) -> Result<bool, WinError> {
    guard.check()?;
    let r = sys::window_rect(drawer).ok_or_else(|| WinError::Window("GetWindowRect 실패".into()))?;
    let Some(img) = sys::capture_window(drawer) else { return fail("서랍 창을 캡처하지 못했다") };
    dump(&img, &format!("round{round:02}"));
    let cells = grid::find_cells(&img);
    // 선택된 칸은 건너뛴다. 우리가 이미 누른 칸이고, 선택하면 썸네일이 줄어들어 서명이 달라져서
    // 스크롤 뒤에 다시 보이면 새 사진으로 오인된다 (실측).
    let fresh: Vec<&Cell> = cells.iter().filter(|c| !c.selected && !st.seen.contains(&c.sig)).collect();
    if fresh.is_empty() {
        st.stall += 1;
    } else {
        st.stall = 0;
    }
    for c in fresh {
        if st.selected >= limit {
            break;
        }
        let video = grid::is_video_tile(&img, c);
        if opt.dry_run || (video && opt.skip_videos) {
            // 누르지 않고 지나간다 (진단이거나, 동영상을 보관하지 않는 경우)
            st.seen.insert(c.sig);
            if video {
                st.videos += 1;
            } else if opt.dry_run {
                st.selected += 1;
            }
            continue;
        }
        front(drawer, guard, "서랍 창")?;
        require_focus(drawer, "서랍 창")?;
        let (cx, cy) = c.circle();
        // 썸네일 위로 먼저 올려서 선택 원을 띄운 뒤 누른다
        hover(r.left + c.x + 62, r.top + c.y + 62, guard)?;
        click_at(r.left + cx, r.top + cy, guard)?;
        // 누른 직후에 바로 기록한다. 사이에 확인(`Interrupted`)이 끼면 눌렀는데 세지 않은 칸이 생긴다.
        st.seen.insert(c.sig);
        st.selected += 1;
        if video {
            st.videos += 1;
        }
        guard.sleep(0.25)?;
    }
    if st.selected >= limit || st.stall >= 2 {
        return Ok(true);
    }
    // 아래로 스크롤 (격자 위에서)
    front(drawer, guard, "서랍 창")?;
    guard.move_cursor(r.left + grid::GRID_LEFT + 150, r.top + 380);
    guard.sleep(0.1)?;
    sys::wheel(-3);
    guard.sleep(0.5)?;
    Ok(false)
}

/// 서랍 격자에서 최신 타일부터 `limit` 개를 고른다. 마우스 때문에 일시정지하면 그 라운드를 화면부터 다시 읽는다.
fn select_photos(drawer: Hwnd, limit: usize, opt: &Options, guard: &Guard) -> Result<Selection, WinError> {
    let mut st = SelState::default();
    for round in 0..400 {
        match select_round(drawer, limit, opt, guard, &mut st, round) {
            Ok(true) => break,
            Ok(false) => {}
            Err(WinError::Interrupted) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(Selection { selected: st.selected, seen: st.seen.len(), videos: st.videos })
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
    guard.set_mouse_mode(PauseMode::Redo); // 마우스를 쓰는 작업: 일시정지하면 단계를 처음부터 다시 한다
    let res = run(save_dir, guard, opt, &info, &mut meta, &mut drawer).map_err(|e| match e {
        WinError::Interrupted => WinError::Collect("마우스가 움직여서 작업을 이어가지 못했다".into()),
        other => other,
    });
    guard.set_mouse_mode(PauseMode::Continue);
    guard.clear_expected();
    if let Some(d) = drawer {
        if sys::is_window(d) {
            sys::close_window(d); // 서랍 창을 남기지 않는다
        }
    }
    let restore = window::restore(title, &info);
    res?;
    meta["window"]["restore"] = restore;
    meta["pauses"] = Value::from(guard.pause_count()); // 사용자가 마우스를 움직여 일시정지한 횟수
    Ok(meta)
}

fn warn(meta: &mut Value, msg: &str) {
    meta["warnings"].as_array_mut().expect("warnings 는 배열").push(Value::from(msg));
}

fn run(save_dir: &Path, guard: &Guard, opt: &Options, info: &window::Info, meta: &mut Value, drawer_out: &mut Option<Hwnd>) -> Result<(), WinError> {
    let chat = info.hwnd;
    let pid = sys::pid_of(chat);
    let before = listing(save_dir);
    let drawer = with_resume(|attempt| {
        if attempt > 0 {
            // 사용자가 끼어든 뒤: 이미 열렸으면 그대로 쓰고, 아니면 열려 있을 수 있는 메뉴를 닫고 처음부터 연다
            if let Some(d) = sys::find_top_level(CHAT_CLASS, DRAWER_TITLE).first().copied() {
                return Ok(d);
            }
            if sys::foreground() == chat {
                let _ = key(sys::VK_ESCAPE, guard);
                let _ = key(sys::VK_ESCAPE, guard);
            }
        }
        open_drawer(chat, guard)
    })?;
    *drawer_out = Some(drawer);
    with_resume(|_| {
        front(drawer, guard, "서랍 창")?;
        guard.sleep(0.6)
    })?;

    let sel = select_photos(drawer, opt.newest.unwrap_or(usize::MAX), opt, guard)?;
    meta["selected"] = Value::from(sel.selected);
    meta["tiles_seen"] = Value::from(sel.seen);
    meta["video_tiles"] = Value::from(sel.videos);
    if opt.dry_run {
        return Ok(()); // 진단: 아무것도 누르지 않았으므로 저장하지 않는다
    }
    if sel.selected == 0 {
        return fail("서랍에서 사진을 찾지 못했다 (사진이 없거나 화면 구성이 다르다)");
    }
    if let Some(n) = opt.newest {
        if sel.selected < n {
            warn(meta, &format!("최신 {n}개를 요청했지만 서랍에서 {}개만 찾았다", sel.selected));
        }
    }

    // 다운로드 아이콘: 선택 바의 모양을 확인한 뒤에만 누른다. 포커스와 모양 확인, 클릭이 한 단계다
    // (사용자가 끼어들었으면 처음부터 다시 확인한다).
    let known: HashSet<Hwnd> = with_resume(|_| {
        front(drawer, guard, "서랍 창")?;
        guard.sleep(0.4)?;
        let Some(img) = sys::capture_window(drawer) else { return fail("서랍 창을 캡처하지 못했다") };
        dump(&img, "before_download");
        if !grid::selection_bar_visible(&img) {
            return fail("선택 바의 모양이 예상과 달라 다운로드를 누르지 않았다");
        }
        let known: HashSet<Hwnd> = sys::top_level_windows(pid).into_iter().map(|d| d.0).collect();
        let r = sys::window_rect(drawer).ok_or_else(|| WinError::Window("GetWindowRect 실패".into()))?;
        let (dx, dy) = grid::download_icon(&img);
        require_focus(drawer, "서랍 창")?;
        click_at(r.left + dx, r.top + dy, guard)?;
        Ok(known)
    })?;

    // 저장 팝업 (별도 창, 제목 없음, 300x200). 저장 중에는 `파일 저장`(진행 막대, `취소`)이고, 끝나면 `저장 결과`
    // (`폴더 열기`)로 바뀐다. 저장 중에 `Esc` 를 누르면 저장이 취소되므로 결과로 바뀐 뒤에만 닫는다.
    with_resume(|attempt| handle_popup(pid, &known, guard, meta, attempt))?;

    // 저장 폴더의 새 파일로 확인한다
    let expected = opt.expect_files.unwrap_or(sel.selected);
    let new_files = |dir: &Path| -> Vec<String> {
        let now = listing(dir);
        let mut v: Vec<String> = now.keys().filter(|k| !before.contains_key(*k)).cloned().collect();
        v.sort();
        v
    };
    let ok = with_resume(|_| guard.wait_until(|| (new_files(save_dir).len() >= expected).then_some(()), 20.0, 0.3))?;
    let files = new_files(save_dir);
    meta["saved_files"] = json!(files);
    meta["seen_cells"] = Value::from(sel.seen);
    if ok.is_none() {
        warn(meta, &format!("저장된 새 파일이 {}개뿐이다 (기대 {expected}개). 저장 폴더가 기본 위치가 아니거나 저장이 중단됐을 수 있다", files.len()));
    }
    Ok(())
}

/// 저장 팝업을 기다렸다가 결과가 되면 `Esc` 로 닫는다. 단계를 처음부터 다시 해도 안전하다
/// (이미 닫혔으면 아무것도 하지 않는다).
fn handle_popup(pid: u32, known: &HashSet<Hwnd>, guard: &Guard, meta: &mut Value, attempt: u32) -> Result<(), WinError> {
    let popup = guard.wait_until(
        || {
            sys::top_level_windows(pid)
                .into_iter()
                .find(|(h, cls, t)| cls == CHAT_CLASS && t.is_empty() && !known.contains(h))
                .map(|d| d.0)
        },
        if attempt == 0 { 8.0 } else { 1.0 },
        0.2,
    )?;
    let Some(p) = popup else {
        if attempt == 0 {
            warn(meta, "저장 결과 팝업을 찾지 못했다. 열려 있다면 직접 닫아야 한다");
        }
        return Ok(());
    };
    // 끝났다는 판단은 두 번 연속 확인되어야 한다. 팝업이 막 떴을 때는 캡처가 실패할 수 있는데(실측) 그것을
    // 끝난 것으로 보면 저장 중에 Esc 를 눌러 저장이 취소된다. 캡처 실패는 "모름"이라 계속 기다린다.
    let (mut calm, mut unknown) = (0u32, 0u32);
    let finished = guard.wait_until(
        || match sys::capture_window(p) {
            Some(img) => {
                unknown = 0;
                if grid::save_in_progress(&img) {
                    calm = 0;
                    None
                } else {
                    calm += 1;
                    (calm >= 2).then_some(())
                }
            }
            None => {
                unknown += 1;
                (unknown >= 50).then_some(()) // 계속 캡처할 수 없으면 더 기다릴 근거가 없다
            }
        },
        300.0,
        0.4,
    )?;
    if finished.is_none() {
        return fail("사진 저장이 5분 안에 끝나지 않았다 (팝업을 닫지 않았다)");
    }
    guard.sleep(0.5)?;
    if let Some(img) = sys::capture_window(p) {
        dump(&img, "popup_result");
    }
    if sys::foreground() != p && !sys::bring_to_front(p, guard, 2.0)? {
        warn(meta, "저장 결과 팝업에 포커스를 주지 못해 닫지 않았다");
    } else if sys::foreground() == p {
        key(sys::VK_ESCAPE, guard)?;
        if guard.wait_until(|| (!sys::is_visible(p)).then_some(()), 3.0, 0.1)?.is_none() {
            warn(meta, "Esc 를 보냈지만 저장 결과 팝업이 닫히지 않았다");
        }
    }
    Ok(())
}
