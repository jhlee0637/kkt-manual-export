//! 설정 파일. 마법사가 바꾼 설정을 기억한다. 위치는 화면 맨 위에 항상 보여 준다.
//!
//! 위치 (환경변수 `KKT_CONFIG` 로 파일 경로를 직접 지정할 수 있다):
//! - Windows: `%APPDATA%\kkt-manual-export\config.json`
//! - macOS:   `~/Library/Application Support/kkt-manual-export/config.json`
//! - 그 밖:   `$XDG_CONFIG_HOME` 또는 `~/.config` 아래 `kkt-manual-export/config.json`

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "kkt-manual-export";
pub const FOLDER_NAME: &str = "kkt-manual-export-archive";

/// 동영상을 아카이브에 보관할지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Videos {
    /// 동영상이 있을 때마다 묻는다 (기본)
    Ask,
    /// 항상 보관한다
    Keep,
    /// 보관하지 않는다 (서랍에서 동영상은 받지도 않는다)
    Skip,
}

impl Videos {
    pub fn as_str(self) -> &'static str {
        match self {
            Videos::Ask => "ask",
            Videos::Keep => "keep",
            Videos::Skip => "skip",
        }
    }

    pub fn parse(s: &str) -> Option<Videos> {
        match s {
            "ask" => Some(Videos::Ask),
            "keep" => Some(Videos::Keep),
            "skip" => Some(Videos::Skip),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Videos::Ask => "그때그때 물어보기",
            Videos::Keep => "항상 보관",
            Videos::Skip => "보관하지 않기",
        }
    }

    /// 설정 화면에서 누를 때마다 다음 값으로 돌린다.
    pub fn next(self) -> Videos {
        match self {
            Videos::Ask => Videos::Keep,
            Videos::Keep => Videos::Skip,
            Videos::Skip => Videos::Ask,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// 정리 결과(`archive\`)와 내보내기 TXT(`exports\`)를 두는 폴더
    pub output_dir: PathBuf,
    /// 카카오톡이 사진을 저장하는 폴더. 카카오톡 설정의 저장 위치와 같아야 새 파일을 확인할 수 있다 (우리는 지켜보기만 한다)
    pub kakao_photo_dir: PathBuf,
    pub videos: Videos,
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

pub fn default_output_dir() -> PathBuf {
    home_dir().join("Downloads").join(FOLDER_NAME)
}

pub fn default_kakao_photo_dir() -> PathBuf {
    home_dir().join("Documents").join("카카오톡 받은 파일")
}

impl Default for Settings {
    fn default() -> Self {
        Settings { output_dir: default_output_dir(), kakao_photo_dir: default_kakao_photo_dir(), videos: Videos::Ask }
    }
}

/// 설정 파일 경로 규칙 (순수 함수). `get` 은 환경변수 조회.
pub fn config_path_from(os: &str, get: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(p) = get("KKT_CONFIG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let home = || PathBuf::from(get("USERPROFILE").or_else(|| get("HOME")).unwrap_or_default());
    let dir = match os {
        "windows" => get("APPDATA").filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming")),
        "macos" => home().join("Library").join("Application Support"),
        _ => get("XDG_CONFIG_HOME").filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config")),
    };
    dir.join(APP_DIR).join("config.json")
}

pub fn config_path() -> PathBuf {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "other"
    };
    config_path_from(os, |k| std::env::var(k).ok())
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoadStatus {
    /// 파일이 없어서 기본값으로 새로 만들었다
    Created,
    Loaded,
    /// 파일이 있지만 읽을 수 없다. 덮어쓰지 않고 이번 실행은 기본값을 쓴다.
    Invalid(String),
}

impl Settings {
    /// 비어 있거나 잘못된 항목은 기본값으로 둔다. 모르는 항목은 무시한다.
    pub fn from_json(v: &Value) -> Settings {
        let d = Settings::default();
        let path = |k: &str, dflt: PathBuf| v.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(PathBuf::from).unwrap_or(dflt);
        Settings {
            output_dir: path("output_dir", d.output_dir),
            kakao_photo_dir: path("kakao_photo_dir", d.kakao_photo_dir),
            videos: v.get("videos").and_then(Value::as_str).and_then(Videos::parse).unwrap_or(d.videos),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "output_dir": self.output_dir.to_string_lossy(),
            "kakao_photo_dir": self.kakao_photo_dir.to_string_lossy(),
            "videos": self.videos.as_str(),
        })
    }

    pub fn load(path: &Path) -> (Settings, LoadStatus) {
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
                Ok(v) if v.is_object() => (Settings::from_json(&v), LoadStatus::Loaded),
                Ok(_) => (Settings::default(), LoadStatus::Invalid("최상위가 객체({...})가 아니다".into())),
                Err(e) => (Settings::default(), LoadStatus::Invalid(e.to_string())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let s = Settings::default();
                match s.save(path) {
                    Ok(()) => (s, LoadStatus::Created),
                    Err(e) => (s, LoadStatus::Invalid(format!("설정 파일을 만들지 못했다: {e}"))),
                }
            }
            Err(e) => (Settings::default(), LoadStatus::Invalid(e.to_string())),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&self.to_json()).expect("직렬화");
        // 쓰는 도중 꺼져도 기존 파일이 깨지지 않게 임시 파일에 쓴 뒤 바꾼다
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, format!("{text}\n"))?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn config_path_rules_per_os() {
        let w = config_path_from("windows", env(&[("APPDATA", r"C:\Users\a\AppData\Roaming"), ("USERPROFILE", r"C:\Users\a")]));
        assert_eq!(w, PathBuf::from(r"C:\Users\a\AppData\Roaming").join("kkt-manual-export").join("config.json"));
        let w2 = config_path_from("windows", env(&[("USERPROFILE", r"C:\Users\a")]));
        assert!(w2.starts_with(PathBuf::from(r"C:\Users\a").join("AppData").join("Roaming")));
        let m = config_path_from("macos", env(&[("HOME", "/Users/a")]));
        assert_eq!(m, PathBuf::from("/Users/a/Library/Application Support/kkt-manual-export/config.json"));
        let x = config_path_from("other", env(&[("HOME", "/home/a")]));
        assert_eq!(x, PathBuf::from("/home/a/.config/kkt-manual-export/config.json"));
        let x2 = config_path_from("other", env(&[("HOME", "/home/a"), ("XDG_CONFIG_HOME", "/cfg")]));
        assert_eq!(x2, PathBuf::from("/cfg/kkt-manual-export/config.json"));
    }

    #[test]
    fn env_override_wins() {
        let p = config_path_from("windows", env(&[("KKT_CONFIG", "/tmp/my.json"), ("APPDATA", "/x")]));
        assert_eq!(p, PathBuf::from("/tmp/my.json"));
    }

    #[test]
    fn json_round_trip_and_defaults() {
        let s = Settings { output_dir: PathBuf::from("/data/out"), kakao_photo_dir: PathBuf::from("/data/kakao"), videos: Videos::Keep };
        assert_eq!(Settings::from_json(&s.to_json()), s);
        let d = Settings::from_json(&json!({}));
        assert_eq!(d, Settings::default());
        let partial = Settings::from_json(&json!({"videos": "skip", "output_dir": "  ", "unknown": 1, "kakao_photo_dir": 5}));
        assert_eq!(partial.videos, Videos::Skip);
        assert_eq!(partial.output_dir, Settings::default().output_dir, "빈 경로는 기본값");
        assert_eq!(partial.kakao_photo_dir, Settings::default().kakao_photo_dir, "문자열이 아니면 기본값");
        assert_eq!(Settings::from_json(&json!({"videos": "bogus"})).videos, Videos::Ask);
    }

    #[test]
    fn videos_cycle_and_parse() {
        assert_eq!(Videos::Ask.next(), Videos::Keep);
        assert_eq!(Videos::Keep.next(), Videos::Skip);
        assert_eq!(Videos::Skip.next(), Videos::Ask);
        for v in [Videos::Ask, Videos::Keep, Videos::Skip] {
            assert_eq!(Videos::parse(v.as_str()), Some(v));
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kkt-settings-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn load_creates_missing_file_then_reads_it_back() {
        let dir = tmp("create");
        let path = dir.join("sub").join("config.json");
        let (s, st) = Settings::load(&path);
        assert_eq!(st, LoadStatus::Created);
        assert!(path.exists());
        let mut changed = s.clone();
        changed.videos = Videos::Skip;
        changed.output_dir = PathBuf::from("/somewhere/else");
        changed.save(&path).unwrap();
        let (again, st2) = Settings::load(&path);
        assert_eq!(st2, LoadStatus::Loaded);
        assert_eq!(again, changed);
        assert!(!path.with_extension("json.tmp").exists(), "임시 파일은 남지 않는다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_file_is_not_overwritten() {
        let dir = tmp("invalid");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, "{ not json").unwrap();
        let (s, st) = Settings::load(&path);
        assert!(matches!(st, LoadStatus::Invalid(_)));
        assert_eq!(s, Settings::default());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json", "사용자의 파일을 덮어쓰지 않는다");
        fs::write(&path, "[1,2]").unwrap();
        assert!(matches!(Settings::load(&path).1, LoadStatus::Invalid(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bom_is_tolerated() {
        let dir = tmp("bom");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, "\u{feff}{\"videos\": \"keep\"}").unwrap();
        assert_eq!(Settings::load(&path).0.videos, Videos::Keep);
        let _ = fs::remove_dir_all(&dir);
    }
}
