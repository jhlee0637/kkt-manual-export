//! 카카오톡 채팅창을 수집하기 좋은 크기/위치로 맞추고, 끝나면 원래대로 돌린다.
//!
//! 수집은 화면 좌표(≡ 버튼, 서랍 선택 원 등)에 의존한다. ≡ 는 창 기준 오른쪽/위 오프셋이라
//! 크기가 바뀌어도 유효하지만(실측: 380x640, 1085x450), 높이가 작으면 메시지 목록이 좁아진다.
//! → 수집 전에 검증된 크기로 맞추고, 맞추지 못하면 중단한다.

use crate::sys::{self, Hwnd, RawRect};
use crate::WinError;
use serde_json::{json, Value};

pub const CHAT_CLASS: &str = "EVA_Window_Dblclk";
pub const MAIN_WINDOW_TITLE: &str = "카카오톡";
pub const DRAWER_TITLE: &str = "채팅방 서랍";

// 실측으로 ≡ 앵커가 동작한 크기: 380x640. 폭은 그대로 두고 높이는 가능하면 더 크게 쓴다.
pub const DEFAULT_WIDTH: i32 = 380;
pub const DEFAULT_HEIGHT: i32 = 800;
pub const MIN_WIDTH: i32 = 380; // 이보다 좁으면 헤더 아이콘이 겹칠 수 있다 (미검증 → 거부)
pub const MIN_HEIGHT: i32 = 560; // 이보다 낮으면 메시지 목록이 너무 좁다
pub const SIZE_TOLERANCE: i32 = 2; // 카카오톡이 테두리 때문에 1~2px 다르게 적용할 수 있다

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.left + self.width
    }
    pub fn bottom(&self) -> i32 {
        self.top + self.height
    }
    fn from_raw(r: RawRect) -> Rect {
        Rect { left: r.left, top: r.top, width: r.right - r.left, height: r.bottom - r.top }
    }
    fn to_json(self) -> Value {
        json!({"left": self.left, "top": self.top, "width": self.width, "height": self.height})
    }
}

/// 목표 사각형을 계산한다 (순수 함수).
/// 크기는 작업 영역에 들어가도록 줄이되 최소 크기보다 작아지면 오류. 위치는 지정이 없으면 현재 왼쪽 위를
/// 유지하고, 작업 영역을 벗어나면 안쪽으로 민다.
pub fn plan_rect(current: Rect, work: Rect, width: i32, height: i32, pos: Option<(i32, i32)>, margin: i32) -> Result<Rect, WinError> {
    if width < MIN_WIDTH || height < MIN_HEIGHT {
        return Err(WinError::Window(format!("요청 크기 {width}x{height}가 최소 {MIN_WIDTH}x{MIN_HEIGHT}보다 작다")));
    }
    let w = width.min(work.width - 2 * margin);
    let h = height.min(work.height - 2 * margin);
    if w < MIN_WIDTH || h < MIN_HEIGHT {
        return Err(WinError::Window(format!(
            "작업 영역 {}x{}에 {MIN_WIDTH}x{MIN_HEIGHT} 이상의 창을 둘 수 없다",
            work.width, work.height
        )));
    }
    let (left, top) = pos.unwrap_or((current.left, current.top));
    let left = (work.left + margin).max(left.min(work.right() - margin - w));
    let top = (work.top + margin).max(top.min(work.bottom() - margin - h));
    Ok(Rect { left, top, width: w, height: h })
}

pub fn within_tolerance(actual: Rect, target: Rect) -> bool {
    (actual.width - target.width).abs() <= SIZE_TOLERANCE
        && (actual.height - target.height).abs() <= SIZE_TOLERANCE
        && (actual.left - target.left).abs() <= SIZE_TOLERANCE
        && (actual.top - target.top).abs() <= SIZE_TOLERANCE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Normal,
    Minimized,
    Maximized,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Normal => "normal",
            State::Minimized => "minimized",
            State::Maximized => "maximized",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreAction {
    Minimize,
    Maximize,
    SetRect(Rect),
}

/// 수집 후 창을 원래대로 돌리는 동작 목록 (순수 함수). 최소화/최대화였던 창은 그 상태로 되돌린다
/// (normalize 가 먼저 일반 상태로 복원해서 크기를 쟀다). 일반 상태였던 창은 원래 사각형으로 되돌리되,
/// 이미 같으면 아무것도 하지 않는다.
pub fn plan_restore(state_before: State, rect_before: Rect, rect_now: Rect) -> Vec<RestoreAction> {
    match state_before {
        State::Minimized => vec![RestoreAction::Minimize],
        State::Maximized => vec![RestoreAction::Maximize],
        State::Normal if within_tolerance(rect_now, rect_before) => vec![],
        State::Normal => vec![RestoreAction::SetRect(rect_before)],
    }
}

pub fn find_chat_windows(title: &str) -> Result<Vec<Hwnd>, WinError> {
    if !sys::SUPPORTED {
        return Err(WinError::Window("Windows 에서만 동작한다".into()));
    }
    if title == MAIN_WINDOW_TITLE {
        return Err(WinError::Window("메인 창은 대상이 아니다".into()));
    }
    Ok(sys::find_top_level(CHAT_CLASS, title))
}

/// 수집할 수 있는 채팅창 제목 목록 (중복 제거, 정렬). 메인 창·서랍·제목 없는 팝업은 제외한다.
pub fn list_chat_titles() -> Vec<String> {
    let mut v: Vec<String> = sys::list_top_level(CHAT_CLASS)
        .into_iter()
        .map(|(_, t)| t)
        .filter(|t| is_chat_title(t))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// 채팅방 제목으로 볼 수 있는 창 제목인지 (같은 클래스를 쓰는 메인 창, 서랍, 제목 없는 팝업을 거른다).
pub fn is_chat_title(t: &str) -> bool {
    !t.is_empty() && t != MAIN_WINDOW_TITLE && t != DRAWER_TITLE
}

fn get_rect(h: Hwnd) -> Result<Rect, WinError> {
    sys::window_rect(h).map(Rect::from_raw).ok_or_else(|| WinError::Window("GetWindowRect 실패".into()))
}

fn state_of(h: Hwnd) -> State {
    if sys::is_iconic(h) {
        State::Minimized
    } else if sys::is_zoomed(h) {
        State::Maximized
    } else {
        State::Normal
    }
}

/// 정규화 전의 상태. `restore` 에 그대로 넘긴다.
pub struct Info {
    pub hwnd: Hwnd,
    pub state_before: State,
    pub before: Rect,
    pub target: Rect,
    pub changed: bool,
}

impl Info {
    pub fn summary(&self) -> Value {
        json!({"state_before": self.state_before.as_str(), "before": self.before.to_json()})
    }
}

pub fn normalize(title: &str, width: i32, height: i32, pos: Option<(i32, i32)>) -> Result<Info, WinError> {
    sys::set_dpi_aware();
    let hs = find_chat_windows(title)?;
    let hwnd = match hs.as_slice() {
        [] => return Err(WinError::Window(format!("제목이 {title:?}인 채팅창이 없다 (창으로 열려 있어야 한다)"))),
        [h] => *h,
        many => return Err(WinError::Window(format!("제목이 {title:?}인 창이 {}개다. 대상을 특정할 수 없어 중단한다", many.len()))),
    };
    let state = state_of(hwnd);
    if state != State::Normal {
        sys::show_window(hwnd, sys::SW_RESTORE);
    }
    let before = get_rect(hwnd)?;
    let work = sys::work_area(hwnd).map(Rect::from_raw).ok_or_else(|| WinError::Window("GetMonitorInfo 실패".into()))?;
    let target = plan_rect(before, work, width, height, pos, 8)?;
    let mut changed = false;
    if !within_tolerance(before, target) {
        if !sys::set_window_rect(hwnd, target.left, target.top, target.width, target.height) {
            return Err(WinError::Window("SetWindowPos 실패".into()));
        }
        changed = true;
    }
    let after = get_rect(hwnd)?;
    if !within_tolerance(after, target) {
        return Err(WinError::Window(format!(
            "창 크기를 맞추지 못했다: 목표 {}x{}, 실제 {}x{}",
            target.width, target.height, after.width, after.height
        )));
    }
    Ok(Info { hwnd, state_before: state, before, target, changed })
}

/// normalize 이전의 크기/위치/상태로 되돌린다. 실패해도 오류를 던지지 않고 결과에 남긴다
/// (중단·오류 처리 중에 호출되므로 원래 오류를 가리면 안 된다).
pub fn restore(title: &str, info: &Info) -> Value {
    sys::set_dpi_aware();
    let hs = match find_chat_windows(title) {
        Ok(h) => h,
        Err(e) => return json!({"restored": false, "actions": [], "warning": format!("복원 중 오류: {e}")}),
    };
    if hs.len() != 1 {
        return json!({"restored": false, "actions": [], "warning": format!("복원할 창을 특정하지 못했다 (일치 {}개)", hs.len())});
    }
    let hwnd = hs[0];
    let mut actions: Vec<&str> = Vec::new();
    let now = match get_rect(hwnd) {
        Ok(r) => r,
        Err(e) => return json!({"restored": false, "actions": [], "warning": format!("복원 중 오류: {e}")}),
    };
    for act in plan_restore(info.state_before, info.before, now) {
        match act {
            RestoreAction::Minimize => {
                actions.push("minimize");
                sys::show_window(hwnd, sys::SW_MINIMIZE);
            }
            RestoreAction::Maximize => {
                actions.push("maximize");
                sys::show_window(hwnd, sys::SW_MAXIMIZE);
            }
            RestoreAction::SetRect(r) => {
                actions.push("set_rect");
                sys::set_window_rect(hwnd, r.left, r.top, r.width, r.height);
            }
        }
    }
    let restored = if info.state_before == State::Normal {
        get_rect(hwnd).map(|r| within_tolerance(r, info.before)).unwrap_or(false)
    } else {
        true
    };
    json!({"restored": restored, "actions": actions})
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { left: 0, top: 0, width: 1920, height: 1040 };

    fn r(left: i32, top: i32, width: i32, height: i32) -> Rect {
        Rect { left, top, width, height }
    }

    #[test]
    fn plan_keeps_position_and_sets_size() {
        assert_eq!(plan_rect(r(100, 50, 500, 450), WORK, 380, 800, None, 8).unwrap(), r(100, 50, 380, 800));
    }

    #[test]
    fn plan_pushes_inside_work_area() {
        let t = plan_rect(r(1800, 900, 500, 450), WORK, 380, 800, None, 8).unwrap();
        assert_eq!(t, r(1532, 232, 380, 800));
    }

    #[test]
    fn plan_shrinks_to_work_area_but_not_below_minimum() {
        let small = r(0, 0, 1920, 700);
        assert_eq!(plan_rect(r(0, 0, 1, 1), small, 380, 800, None, 8).unwrap().height, 684);
        assert!(plan_rect(r(0, 0, 1, 1), r(0, 0, 1920, 560), 380, 800, None, 8).is_err());
        assert!(plan_rect(r(0, 0, 1, 1), WORK, 300, 800, None, 8).is_err());
        assert!(plan_rect(r(0, 0, 1, 1), WORK, 380, 500, None, 8).is_err());
    }

    #[test]
    fn plan_uses_explicit_position() {
        assert_eq!(plan_rect(r(0, 0, 1, 1), WORK, 380, 800, Some((300, 20)), 8).unwrap(), r(300, 20, 380, 800));
    }

    #[test]
    fn chat_title_filter() {
        assert!(is_chat_title("test-2"));
        assert!(!is_chat_title(""));
        assert!(!is_chat_title("카카오톡"));
        assert!(!is_chat_title("채팅방 서랍"));
    }

    #[test]
    fn tolerance_is_inclusive() {
        assert!(within_tolerance(r(0, 0, 382, 800), r(0, 0, 380, 800)));
        assert!(!within_tolerance(r(0, 0, 383, 800), r(0, 0, 380, 800)));
    }

    #[test]
    fn restore_plans() {
        let b = r(10, 20, 500, 450);
        assert_eq!(plan_restore(State::Minimized, b, r(0, 0, 380, 800)), vec![RestoreAction::Minimize]);
        assert_eq!(plan_restore(State::Maximized, b, r(0, 0, 380, 800)), vec![RestoreAction::Maximize]);
        assert_eq!(plan_restore(State::Normal, b, r(10, 20, 501, 451)), vec![]);
        assert_eq!(plan_restore(State::Normal, b, r(0, 0, 380, 800)), vec![RestoreAction::SetRect(b)]);
    }
}
