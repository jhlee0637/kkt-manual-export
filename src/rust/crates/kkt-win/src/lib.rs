//! 카카오톡 채팅창을 내보내기(Ctrl+S)로 수집하는 계층. Windows 에서만 동작한다.
//! 순수 로직(`plan_rect`, `plan_restore`, `Guard`)은 모든 OS 에서 시험할 수 있다.

pub mod collect;
pub mod guard;
pub mod sys;
pub mod window;

use std::fmt;

#[derive(Debug)]
pub enum WinError {
    /// 사용자가 Ctrl+D 로 중단했다.
    Aborted,
    Collect(String),
    Window(String),
}

impl fmt::Display for WinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WinError::Aborted => write!(f, "Ctrl+D 로 중단됨"),
            WinError::Collect(m) | WinError::Window(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for WinError {}
