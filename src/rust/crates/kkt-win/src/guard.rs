//! 긴급 탈출(Ctrl+D)과 마우스 감지 일시정지.
//!
//! **Ctrl+D**: 전역 단축키로 등록하지 않는다. 등록하면 봇이 도는 동안 모든 프로그램의 Ctrl+D 가 가로채진다.
//! 대신 GetAsyncKeyState 로 폴링해서 키를 소비하지 않고 감지한다.
//! 봇은 VK_D 를 주입하지 않는다 (파일 경로는 유니코드 메시지로 넣는다).
//! 모든 대기는 `Guard::sleep`/`wait_until` 을 거치므로 대기 중에도 감지된다.
//!
//! **마우스 감지**: 봇이 마우스를 옮길 때마다 기대 위치를 기록하고(`move_cursor`), 확인할 때마다 커서가 그 자리에
//! (오차 안에) 있는지 본다. 벗어나면 사용자가 움직인 것이므로 일시정지하고, 마우스가 `idle` 초 동안 멈추면 이어간다.
//! 일시정지 중에도 Ctrl+D 는 먹는다.

use crate::{sys, WinError};
use std::cell::Cell;
use std::thread;
use std::time::{Duration, Instant};

/// 마우스가 기대 위치에서 이만큼 벗어나면 사용자가 움직인 것으로 본다 (px). 손떨림과 반올림은 허용한다.
pub const MOUSE_TOLERANCE: i32 = 5;
/// 마우스가 이만큼 멈춰 있으면 이어간다 (초).
pub const MOUSE_IDLE_SECS: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseMode {
    /// 일시정지가 끝나면 그대로 이어간다 (키보드만 쓰는 작업).
    Continue,
    /// 일시정지가 끝나면 `Interrupted` 를 돌려 호출한 쪽이 현재 단계를 처음부터 다시 하게 한다 (마우스를 쓰는 작업).
    Redo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseEvent {
    Started,
    Resumed,
}

pub struct Guard {
    pressed: Box<dyn Fn() -> bool>,
    cursor: Box<dyn Fn() -> (i32, i32)>,
    notify: Box<dyn Fn(PauseEvent)>,
    aborted: Cell<bool>,
    expected: Cell<Option<(i32, i32)>>,
    mode: Cell<PauseMode>,
    pauses: Cell<u32>,
    idle: f64,
}

impl Guard {
    pub fn new() -> Self {
        let mut g = Self::with_pressed(sys::ctrl_d_pressed);
        g.notify = Box::new(|e| match e {
            PauseEvent::Started => eprintln!(
                "\n[일시정지] 마우스가 움직여서 멈췄습니다. 마우스에서 손을 떼면 {:.0}초 뒤에 이어갑니다. (중단: Ctrl+D)",
                MOUSE_IDLE_SECS
            ),
            PauseEvent::Resumed => eprintln!("[재개] 이어서 진행합니다."),
        });
        g
    }

    pub fn with_pressed(f: impl Fn() -> bool + 'static) -> Self {
        Guard {
            pressed: Box::new(f),
            cursor: Box::new(sys::cursor_pos),
            notify: Box::new(|_| {}),
            aborted: Cell::new(false),
            expected: Cell::new(None),
            mode: Cell::new(PauseMode::Continue),
            pauses: Cell::new(0),
            idle: MOUSE_IDLE_SECS,
        }
    }

    /// 시험용: 커서 위치, 알림, 멈춤 판단 시간을 바꾼다.
    pub fn with_mouse(mut self, cursor: impl Fn() -> (i32, i32) + 'static, notify: impl Fn(PauseEvent) + 'static, idle: f64) -> Self {
        self.cursor = Box::new(cursor);
        self.notify = Box::new(notify);
        self.idle = idle;
        self
    }

    pub fn set_mouse_mode(&self, mode: PauseMode) {
        self.mode.set(mode);
    }

    /// 지금까지 마우스 때문에 일시정지한 횟수.
    pub fn pause_count(&self) -> u32 {
        self.pauses.get()
    }

    /// 봇이 마우스를 `(x, y)` 로 옮긴다. 이 위치가 이후 확인의 기준이 된다.
    pub fn move_cursor(&self, x: i32, y: i32) {
        self.expected.set(Some((x, y)));
        sys::cursor_to(x, y);
    }

    /// 마우스를 움직이지 않는 작업에서, 지금 위치를 기준으로 삼는다 (사용자가 움직이면 일시정지).
    pub fn track_current(&self) {
        self.expected.set(Some((self.cursor)()));
    }

    /// 기준을 지운다 (마우스 감지를 끈다).
    pub fn clear_expected(&self) {
        self.expected.set(None);
    }

    fn check_abort(&self) -> Result<(), WinError> {
        if self.aborted.get() || (self.pressed)() {
            self.aborted.set(true);
            return Err(WinError::Aborted);
        }
        Ok(())
    }

    pub fn check(&self) -> Result<(), WinError> {
        self.check_abort()?;
        if let Some((ex, ey)) = self.expected.get() {
            let (x, y) = (self.cursor)();
            if (x - ex).abs() > MOUSE_TOLERANCE || (y - ey).abs() > MOUSE_TOLERANCE {
                return self.pause((x, y));
            }
        }
        Ok(())
    }

    /// 마우스가 `idle` 초 동안 멈출 때까지 기다린다. Ctrl+D 는 계속 먹는다.
    fn pause(&self, start: (i32, i32)) -> Result<(), WinError> {
        self.pauses.set(self.pauses.get() + 1);
        (self.notify)(PauseEvent::Started);
        let mut last = start;
        let mut since = Instant::now();
        loop {
            self.check_abort()?;
            thread::sleep(Duration::from_millis(30));
            let now = (self.cursor)();
            if now != last {
                last = now;
                since = Instant::now();
            }
            if since.elapsed().as_secs_f64() >= self.idle {
                break;
            }
        }
        (self.notify)(PauseEvent::Resumed);
        match self.mode.get() {
            PauseMode::Continue => {
                self.expected.set(Some(last)); // 새 위치를 기준으로 이어간다
                Ok(())
            }
            PauseMode::Redo => {
                self.expected.set(None); // 봇이 다음에 마우스를 옮길 때 다시 기록한다
                Err(WinError::Interrupted)
            }
        }
    }

    pub fn sleep(&self, seconds: f64) -> Result<(), WinError> {
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        loop {
            self.check()?;
            let now = Instant::now();
            if now >= end {
                return Ok(());
            }
            thread::sleep((end - now).min(Duration::from_millis(30)));
        }
    }

    /// `cond` 가 `Some` 을 돌려줄 때까지 기다린다. 시간 초과면 `Ok(None)`.
    pub fn wait_until<T>(&self, mut cond: impl FnMut() -> Option<T>, timeout: f64, step: f64) -> Result<Option<T>, WinError> {
        let end = Instant::now() + Duration::from_secs_f64(timeout);
        loop {
            self.check()?;
            if let Some(v) = cond() {
                return Ok(Some(v));
            }
            if Instant::now() >= end {
                return Ok(None);
            }
            thread::sleep(Duration::from_secs_f64(step));
        }
    }
}

impl Default for Guard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn check_passes_until_pressed_then_stays_aborted() {
        let flag = Rc::new(Cell::new(false));
        let f = flag.clone();
        let g = Guard::with_pressed(move || f.get());
        assert!(g.check().is_ok());
        flag.set(true);
        assert!(matches!(g.check(), Err(WinError::Aborted)));
        flag.set(false);
        assert!(matches!(g.check(), Err(WinError::Aborted)), "한 번 중단되면 계속 중단 상태");
    }

    #[test]
    fn sleep_is_interrupted() {
        let g = Guard::with_pressed(|| true);
        assert!(matches!(g.sleep(10.0), Err(WinError::Aborted)));
    }

    #[test]
    fn wait_until_returns_value_or_none_on_timeout() {
        let g = Guard::with_pressed(|| false);
        let mut n = 0;
        let v = g.wait_until(|| { n += 1; (n == 3).then_some(n) }, 1.0, 0.001).unwrap();
        assert_eq!(v, Some(3));
        assert_eq!(g.wait_until(|| None::<u8>, 0.02, 0.005).unwrap(), None);
    }

    #[test]
    fn wait_until_is_interrupted() {
        let g = Guard::with_pressed(|| true);
        assert!(g.wait_until(|| Some(1), 1.0, 0.001).is_err());
    }

    fn mouse_guard(pos: Rc<Cell<(i32, i32)>>, events: Rc<RefCell<Vec<PauseEvent>>>) -> Guard {
        let ev = events.clone();
        Guard::with_pressed(|| false).with_mouse(move || pos.get(), move |e| ev.borrow_mut().push(e), 0.06)
    }

    #[test]
    fn no_pause_without_expectation_or_within_tolerance() {
        let pos = Rc::new(Cell::new((100, 100)));
        let events = Rc::new(RefCell::new(Vec::new()));
        let g = mouse_guard(pos.clone(), events.clone());
        pos.set((900, 900));
        assert!(g.check().is_ok(), "기준이 없으면 마우스를 보지 않는다");
        g.track_current();
        pos.set((900 + MOUSE_TOLERANCE, 900 - MOUSE_TOLERANCE));
        assert!(g.check().is_ok(), "오차 안의 움직임은 무시한다");
        assert!(events.borrow().is_empty() && g.pause_count() == 0);
    }

    #[test]
    fn user_moving_the_mouse_pauses_then_redo_returns_interrupted() {
        let pos = Rc::new(Cell::new((100, 100)));
        let events = Rc::new(RefCell::new(Vec::new()));
        let g = mouse_guard(pos.clone(), events.clone());
        g.set_mouse_mode(PauseMode::Redo);
        g.track_current(); // 봇이 (100,100) 에 둔 것으로 본다
        pos.set((100 + MOUSE_TOLERANCE + 1, 100));
        assert!(matches!(g.check(), Err(WinError::Interrupted)));
        assert_eq!(*events.borrow(), [PauseEvent::Started, PauseEvent::Resumed]);
        assert_eq!(g.pause_count(), 1);
        assert!(g.check().is_ok(), "일시정지 뒤 기준은 지워져서, 봇이 다시 마우스를 옮기기 전까지 괜찮다");
        g.track_current();
        assert!(g.check().is_ok());
    }

    #[test]
    fn continue_mode_resumes_in_place_with_new_baseline() {
        let pos = Rc::new(Cell::new((10, 10)));
        let events = Rc::new(RefCell::new(Vec::new()));
        let g = mouse_guard(pos.clone(), events.clone());
        g.track_current();
        pos.set((300, 300));
        assert!(g.check().is_ok(), "키보드만 쓰는 작업은 멈췄다가 그대로 이어간다");
        assert_eq!(g.pause_count(), 1);
        assert!(g.check().is_ok(), "새 위치가 기준이 되어 다시 멈추지 않는다");
        assert_eq!(g.pause_count(), 1);
    }

    #[test]
    fn pause_waits_until_the_mouse_is_still() {
        // 처음 6번의 조회에서는 마우스가 계속 움직이고, 그 뒤로는 멈춘다
        let calls = Rc::new(Cell::new(0i32));
        let c = calls.clone();
        let cursor = move || {
            c.set(c.get() + 1);
            let n = c.get();
            if n <= 6 { (100 + n * 20, 100) } else { (220, 100) }
        };
        let g = Guard::with_pressed(|| false).with_mouse(cursor, |_| {}, 0.06);
        g.expected.set(Some((100, 100)));
        let start = Instant::now();
        assert!(g.check().is_ok());
        // 움직이는 동안(약 6번 x 30ms)과 멈춘 뒤 idle(60ms)을 모두 기다려야 한다
        assert!(start.elapsed().as_secs_f64() >= 0.06 + 0.03 * 5.0 - 0.04, "{:?}", start.elapsed());
        assert!(calls.get() > 6);
    }

    #[test]
    fn ctrl_d_works_during_a_pause() {
        // 마우스는 계속 움직이지 않지만 기준에서 멀리 있다. 두 번째 조회부터(일시정지 중) Ctrl+D 가 눌린 상태다.
        let asked = Rc::new(Cell::new(0));
        let a = asked.clone();
        let pressed = move || {
            a.set(a.get() + 1);
            a.get() >= 2
        };
        let g = Guard::with_pressed(pressed).with_mouse(|| (400, 400), |_| {}, 5.0);
        g.expected.set(Some((0, 0)));
        assert!(matches!(g.check(), Err(WinError::Aborted)));
    }
}
