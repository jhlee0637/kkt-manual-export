//! 카카오톡 채팅창을 내보내기(Ctrl+S)로 수집하는 계층. Windows 에서만 동작한다.
//! 순수 로직(`plan_rect`, `plan_restore`, `Guard`)은 모든 OS 에서 시험할 수 있다.

pub mod collect;
pub mod grid;
pub mod guard;
pub mod photos;
pub mod sys;
pub mod window;

use std::fmt;

#[derive(Debug)]
pub enum WinError {
    /// 사용자가 Ctrl+D 로 중단했다.
    Aborted,
    /// 사용자가 마우스를 움직여 일시정지했다가 이어가는 중이다. 마우스를 쓰는 작업은 현재 단계를 처음부터 다시 해야 한다
    /// (사용자가 클릭했을 수 있어 화면 상태를 장담할 수 없다). 바깥으로 새어 나가면 안 된다.
    Interrupted,
    Collect(String),
    Window(String),
}

impl fmt::Display for WinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WinError::Aborted => write!(f, "Ctrl+D 로 중단됨"),
            WinError::Interrupted => write!(f, "마우스가 움직여서 작업을 이어가지 못했다"),
            WinError::Collect(m) | WinError::Window(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for WinError {}
