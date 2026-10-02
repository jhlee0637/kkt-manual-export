use std::fmt;

/// 처리를 중단해야 하는 오류. `message` 는 사람용(한국어), `code` 는 구현이 바뀌어도 변하지 않는 계약이다.
#[derive(Debug, Clone)]
pub struct KktError {
    pub code: String,
    pub message: String,
}

impl KktError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        KktError { code: code.to_string(), message: message.into() }
    }
}

impl fmt::Display for KktError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for KktError {}

pub type Result<T> = std::result::Result<T, KktError>;

impl From<std::io::Error> for KktError {
    fn from(e: std::io::Error) -> Self {
        KktError::new("io_error", e.to_string())
    }
}

impl From<serde_json::Error> for KktError {
    fn from(e: serde_json::Error) -> Self {
        KktError::new("bad_log", format!("이벤트 로그를 읽을 수 없다: {e}"))
    }
}
