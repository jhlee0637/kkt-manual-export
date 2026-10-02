//! 긴급 탈출 (Ctrl+D).
//!
//! 전역 단축키로 등록하지 않는다. 등록하면 봇이 도는 동안 모든 프로그램의 Ctrl+D 가 가로채진다.
//! 대신 GetAsyncKeyState 로 폴링해서 키를 소비하지 않고 감지한다.
//! 봇은 VK_D 를 주입하지 않는다 (파일 경로는 유니코드 메시지로 넣는다).
//! 모든 대기는 `Guard::sleep`/`wait_until` 을 거치므로 대기 중에도 감지된다.

use crate::{sys, WinError};
use std::cell::Cell;
use std::thread;
use std::time::{Duration, Instant};

pub struct Guard {
    pressed: Box<dyn Fn() -> bool>,
    aborted: Cell<bool>,
}

impl Guard {
    pub fn new() -> Self {
        Self::with_pressed(sys::ctrl_d_pressed)
    }

    pub fn with_pressed(f: impl Fn() -> bool + 'static) -> Self {
        Guard { pressed: Box::new(f), aborted: Cell::new(false) }
    }

    pub fn check(&self) -> Result<(), WinError> {
        if self.aborted.get() || (self.pressed)() {
            self.aborted.set(true);
            return Err(WinError::Aborted);
        }
        Ok(())
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
}
