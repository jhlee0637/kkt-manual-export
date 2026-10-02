//! 카카오톡 PC '대화 내보내기' TXT 와 저장한 사진을 추가 전용 이벤트 로그로 정리하는 핵심 로직.
//!
//! Python 구현(src/python/kkt/)과 같은 입력에 같은 출력을 내야 한다. 기준은 tests/golden 의 시나리오다.

pub mod archive;
pub mod attach;
pub mod difflib;
pub mod error;
pub mod link;
pub mod parse;
pub mod pyfmt;
pub mod reconcile;
pub mod state;

/// 이벤트 로그 스키마 버전. 이보다 오래된 로그는 거부한다.
pub const SCHEMA_VERSION: u64 = 2;
