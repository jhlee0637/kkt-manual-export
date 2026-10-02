//! 채팅방 하나의 대화를 내보내기(Ctrl+S)로 받아 파일로 저장한다.
//!
//! 흐름: 창을 수집용 크기로 맞춤(끝나면 원래대로) → 카카오톡 창을 앞으로 → Ctrl+S
//! → 저장 대화상자를 찾아 파일명 칸에 경로를 넣고 저장 버튼 클릭 (포커스 없이 창 메시지로)
//! → 파일 생성 확인 → 완료 팝업 닫기(Enter)
//!
//! 안전 규칙:
//! - 키는 카카오톡 창이 실제로 포커스일 때만 보낸다. 아니면 중단한다.
//! - Enter 는 완료 팝업 창이 포커스일 때만 보낸다 (입력창으로 가면 작성 중인 글이 전송될 수 있다).
//! - 이미 있는 파일은 덮어쓰지 않는다.
//! - Ctrl+D 로 언제든 중단. 중단/오류 시 열린 저장 대화상자를 닫고 창을 원래대로 돌린다.

use crate::guard::Guard;
use crate::sys::{self, Hwnd};
use crate::window::{self, Info, DEFAULT_HEIGHT, DEFAULT_WIDTH};
use crate::WinError;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Options {
    pub width: i32,
    pub height: i32,
    pub dialog_timeout: f64,
    pub save_timeout: f64,
    /// 디버그: 저장 대화상자가 열린 뒤 N초 대기 (Ctrl+D 중단 시험용)
    pub hold: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options { width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT, dialog_timeout: 8.0, save_timeout: 20.0, hold: 0.0 }
    }
}

fn fail<T>(msg: impl Into<String>) -> Result<T, WinError> {
    Err(WinError::Collect(msg.into()))
}

/// `KakaoTalk_….txt` 꼴의 기본 파일명인지 (정규식 `^KakaoTalk_.+\.txt$`).
fn is_default_name(name: &str) -> bool {
    name.len() > "KakaoTalk_.txt".len() && name.starts_with("KakaoTalk_") && name.ends_with(".txt") && !name.contains('\n')
}

type Desc = (Hwnd, String, String, i32);

fn find_desc(descs: &[Desc], class: &str, parent_class: Option<&str>, ctrl_id: Option<i32>) -> Option<Hwnd> {
    descs
        .iter()
        .find(|(_, c, pc, id)| c == class && parent_class.map_or(true, |p| p == pc) && ctrl_id.map_or(true, |i| i == *id))
        .map(|d| d.0)
}

/// 저장 후 새로 뜬 제목 없는 카카오톡 창(대화 내보내기 완료 팝업). 실측: 별도 최상위 창이다.
fn new_popup(pid: u32, known: &HashSet<Hwnd>) -> Option<Hwnd> {
    sys::top_level_windows(pid)
        .into_iter()
        .find(|(h, cls, title)| cls == window::CHAT_CLASS && title.is_empty() && !known.contains(h))
        .map(|d| d.0)
}

pub fn export_chat(title: &str, out_dir: &Path, guard: &Guard, opt: &Options) -> Result<Value, WinError> {
    fs::create_dir_all(out_dir).map_err(|e| WinError::Collect(format!("저장 폴더를 만들지 못했다: {e}")))?;
    sys::set_dpi_aware();
    let info = window::normalize(title, opt.width, opt.height, None)?;
    let mut meta = json!({"title": title, "warnings": [], "window": info.summary()});
    let mut dialog: Option<Hwnd> = None;
    let res = run(title, out_dir, guard, opt, &info, &mut meta, &mut dialog);
    if let Some(d) = dialog {
        if sys::is_window(d) {
            sys::close_window(d); // 중단/오류 시 대화상자를 남기지 않는다
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

fn run(_title: &str, out_dir: &Path, guard: &Guard, opt: &Options, info: &Info, meta: &mut Value, dialog: &mut Option<Hwnd>) -> Result<(), WinError> {
    let hwnd = info.hwnd;
    let pid = sys::pid_of(hwnd);

    if !sys::top_level_dialogs(pid).is_empty() {
        return fail("카카오톡에 이미 열린 대화상자가 있다. 닫은 뒤 다시 실행하라");
    }
    let known: HashSet<Hwnd> = sys::top_level_windows(pid).into_iter().map(|d| d.0).collect();

    if !sys::bring_to_front(hwnd, guard, 2.0)? {
        return fail("카카오톡 창을 앞으로 가져오지 못했다 (포커스를 확인하지 못해 키를 보내지 않는다)");
    }
    guard.sleep(0.4)?;
    if sys::foreground() != hwnd {
        return fail("키를 보내기 직전에 포커스가 다른 창으로 옮겨졌다");
    }
    sys::tap(sys::VK_S, guard, &[sys::VK_CONTROL])?; // Ctrl+S

    let d = guard.wait_until(|| sys::top_level_dialogs(pid).first().copied(), opt.dialog_timeout, 0.1)?;
    let Some(d) = d else { return fail("저장 대화상자가 열리지 않았다") };
    *dialog = Some(d);
    guard.sleep(0.3)?;
    if opt.hold > 0.0 {
        guard.sleep(opt.hold)?;
    }
    let descs = sys::descendants(d);
    let edit = find_desc(&descs, "Edit", Some("ComboBox"), None);
    let save_btn = find_desc(&descs, "Button", None, Some(1));
    let (Some(edit), Some(save_btn)) = (edit, save_btn) else {
        return fail("저장 대화상자에서 파일명 칸이나 저장 버튼을 찾지 못했다");
    };
    let default_name = sys::text_of(edit).trim().to_string();
    if !is_default_name(&default_name) {
        return fail(format!("기본 파일명이 예상과 다르다: {default_name:?}"));
    }
    let target: PathBuf = out_dir.join(&default_name);
    if target.exists() {
        return fail(format!("이미 있는 파일이다 (덮어쓰지 않음): {}", target.display()));
    }
    let target_str = target.to_str().ok_or_else(|| WinError::Collect("저장 경로가 유니코드가 아니다".into()))?;
    sys::set_text(edit, target_str);
    sys::click_button(save_btn);

    let size_of = |p: &Path| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    if guard.wait_until(|| (size_of(&target) > 0).then_some(()), opt.save_timeout, 0.1)?.is_none() {
        return fail("내보낸 파일이 생성되지 않았다");
    }
    let size = size_of(&target);
    guard.sleep(0.5)?;
    if size_of(&target) != size {
        guard.sleep(1.0)?; // 쓰는 중이면 한 번 더 기다린다
    }
    meta["path"] = Value::from(target_str);
    meta["size"] = Value::from(size_of(&target));
    meta["default_name"] = Value::from(default_name);
    *dialog = None;

    // 완료 팝업은 별도 창이다. 포커스가 그 팝업일 때만 Enter 를 보낸다
    // (입력창으로 Enter 가 가면 작성 중인 글이 전송될 수 있다).
    match guard.wait_until(|| new_popup(pid, &known), 5.0, 0.1)? {
        None => warn(meta, "완료 팝업을 찾지 못했다. 열려 있다면 직접 닫아야 한다"),
        Some(popup) => {
            guard.sleep(0.3)?;
            if sys::foreground() != popup && !sys::bring_to_front(popup, guard, 2.0)? {
                warn(meta, "완료 팝업에 포커스를 주지 못해 Enter 를 보내지 않았다");
            } else if sys::foreground() == popup {
                sys::tap(sys::VK_RETURN, guard, &[])?;
                if guard.wait_until(|| (!sys::is_visible(popup)).then_some(()), 3.0, 0.1)?.is_none() {
                    warn(meta, "Enter 를 보냈지만 완료 팝업이 닫히지 않았다");
                }
            }
        }
    }
    guard.sleep(0.3)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_matches_python_regex() {
        assert!(is_default_name("KakaoTalk_20261002_1033_12_345_group.txt"));
        assert!(!is_default_name("KakaoTalk_.txt"));
        assert!(!is_default_name("other.txt"));
        assert!(!is_default_name("KakaoTalk_a.csv"));
    }

    #[test]
    fn find_desc_filters() {
        let d: Vec<Desc> = vec![(1, "Edit".into(), "Foo".into(), 0), (2, "Edit".into(), "ComboBox".into(), 0), (3, "Button".into(), "".into(), 1)];
        assert_eq!(find_desc(&d, "Edit", Some("ComboBox"), None), Some(2));
        assert_eq!(find_desc(&d, "Button", None, Some(1)), Some(3));
        assert_eq!(find_desc(&d, "Button", None, Some(2)), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn export_fails_cleanly_off_windows() {
        let g = Guard::with_pressed(|| false);
        let dir = std::env::temp_dir().join("kkt-win-test-out");
        let e = export_chat("test", &dir, &g, &Options::default()).unwrap_err();
        assert!(e.to_string().contains("Windows"));
    }
}
