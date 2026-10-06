//! 설정 파일. 마법사가 바꾼 설정을 기억한다. 위치는 화면 맨 위에 항상 보여 준다.
//!
//! 형식은 TOML 의 일부(`키 = 값`, `#` 주석)다. 주석에 선택지를 적어 두어서 파일만 열어 봐도 무엇을 고를 수 있는지 안다.
//! 경로는 작은따옴표('...') 안에 그대로 쓴다 (`\` 를 두 번 쓰지 않아도 된다). 직접 고쳐도 되고, 프로그램이 저장할 때는
//! 주석까지 포함해 파일 전체를 다시 쓴다.
//!
//! 위치 (환경변수 `KKT_CONFIG` 로 파일 경로를 직접 지정할 수 있다):
//! - Windows: `%APPDATA%\kkt-manual-export\config.toml`
//! - macOS:   `~/Library/Application Support/kkt-manual-export/config.toml`
//! - 그 밖:   `$XDG_CONFIG_HOME` 또는 `~/.config` 아래 `kkt-manual-export/config.toml`
//!
//! 예전 버전(0.3.0)의 `config.json`(`output_dir`, `kakao_photo_dir`)이 같은 폴더에 있고 `config.toml` 이 없으면 읽어서 옮긴다.
//! 옛 파일은 지우지 않는다.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "kkt-manual-export";
pub const FOLDER_NAME: &str = "kkt-manual-export-archive";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 동영상을 아카이브에 보관할지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Videos {
    /// 동영상이 있을 때마다 묻는다 (기본)
    Ask,
    /// 항상 보관한다
    Keep,
    /// 보관하지 않는다 (서랍에서 동영상은 다운로드하지도 않는다)
    Skip,
}

impl Videos {
    pub const ALL: [Videos; 3] = [Videos::Ask, Videos::Keep, Videos::Skip];

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

    /// 화면에 보이는 이름.
    pub fn label(self) -> &'static str {
        match self {
            Videos::Ask => "물어보기",
            Videos::Keep => "항상 보관",
            Videos::Skip => "보관 안 함",
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
    /// 결과 폴더: 정리한 대화(`archive\`)와 내보낸 TXT(`exports\`)를 두는 폴더
    pub result_dir: PathBuf,
    /// 다운로드 폴더: 카카오톡이 사진·동영상을 내려받는 폴더. 카카오톡 설정의 '사진 저장 위치'와 같아야 내려받은 파일을 확인할 수 있다
    /// (우리는 그 폴더를 바꾸지 않고 지켜보기만 한다)
    pub download_dir: PathBuf,
    pub videos: Videos,
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

pub fn default_result_dir() -> PathBuf {
    home_dir().join("Downloads").join(FOLDER_NAME)
}

pub fn default_download_dir() -> PathBuf {
    home_dir().join("Documents").join("카카오톡 받은 파일")
}

impl Default for Settings {
    fn default() -> Self {
        Settings { result_dir: default_result_dir(), download_dir: default_download_dir(), videos: Videos::Ask }
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
    dir.join(APP_DIR).join("config.toml")
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
    /// 예전 `config.json` 을 읽어 `config.toml` 로 옮겼다
    Migrated(PathBuf),
    /// 파일을 읽을 수 없다. 덮어쓰지 않고 이번 실행은 기본값을 쓴다.
    Invalid(String),
}

#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    pub status: LoadStatus,
    /// 읽다가 알게 된 것: 알 수 없는 값, 읽지 못한 줄 등. 화면에 보여 준다.
    pub notes: Vec<String>,
}

// ───────── 쓰기 ─────────

/// 값을 TOML 문자열로 쓴다. 작은따옴표가 없으면 리터럴(`'…'`, 이스케이프 없음), 있으면 `"…"` 로 이스케이프한다.
fn quote(s: &str) -> String {
    if s.contains('\'') || s.contains('\n') || s.contains('\r') {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    } else {
        format!("'{s}'")
    }
}

impl Settings {
    /// 주석을 포함한 설정 파일 전체.
    pub fn to_toml(&self) -> String {
        format!(
            "# kkt-manual-export v{VERSION} 설정\n\
             # 이 파일은 직접 고쳐도 됩니다. 고친 뒤 프로그램을 다시 실행하면 적용됩니다.\n\
             # 경로는 작은따옴표('...') 안에 그대로 쓰면 됩니다 (\\ 를 두 번 쓰지 않아도 됩니다).\n\
             \n\
             # 결과 폴더: 정리한 대화(archive\\)와 내보낸 TXT(exports\\)를 두는 폴더\n\
             result_dir = {result}\n\
             \n\
             # 다운로드 폴더: 카카오톡이 사진·동영상을 내려받는 폴더\n\
             # 카카오톡 설정의 '사진 저장 위치'와 같아야 내려받은 파일을 확인할 수 있습니다.\n\
             download_dir = {download}\n\
             \n\
             # 동영상 보관 방식\n\
             #   \"ask\"  = 동영상이 있을 때마다 물어봅니다 (기본)\n\
             #   \"keep\" = 항상 보관합니다\n\
             #   \"skip\" = 보관하지 않습니다 (서랍에서 동영상을 다운로드하지도 않습니다)\n\
             videos = \"{videos}\"\n",
            result = quote(&self.result_dir.to_string_lossy()),
            download = quote(&self.download_dir.to_string_lossy()),
            videos = self.videos.as_str(),
        )
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // 쓰는 도중 꺼져도 기존 파일이 깨지지 않게 임시 파일에 쓴 뒤 바꾼다
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, self.to_toml())?;
        fs::rename(&tmp, path)
    }
}

// ───────── 읽기 ─────────

/// `'...'`(리터럴), `"..."`(이스케이프 `\\` `\"` `\n` `\t` 만 해석, 모르는 `\x` 는 그대로), 따옴표 없는 단어를 읽는다.
/// 값 뒤의 `# 주석` 은 무시한다. 읽은 값과 그 값이 끝난 위치 뒤의 나머지를 돌려준다.
fn parse_value(raw: &str) -> Option<String> {
    let t = raw.trim();
    let mut chars = t.chars();
    match chars.next()? {
        '\'' => {
            let rest = &t[1..];
            let end = rest.find('\'')?;
            let tail = rest[end + 1..].trim();
            (tail.is_empty() || tail.starts_with('#')).then(|| rest[..end].to_string())
        }
        '"' => {
            let mut out = String::new();
            let mut it = t[1..].char_indices();
            while let Some((i, c)) = it.next() {
                match c {
                    '\\' => match it.next() {
                        Some((_, '\\')) => out.push('\\'),
                        Some((_, '"')) => out.push('"'),
                        Some((_, 'n')) => out.push('\n'),
                        Some((_, 't')) => out.push('\t'),
                        Some((_, other)) => {
                            out.push('\\');
                            out.push(other);
                        }
                        None => return None,
                    },
                    '"' => {
                        let tail = t[1 + i + 1..].trim();
                        return (tail.is_empty() || tail.starts_with('#')).then_some(out);
                    }
                    c => out.push(c),
                }
            }
            None
        }
        _ => {
            let word = t.split('#').next().unwrap_or("").trim();
            (!word.is_empty()).then(|| word.to_string())
        }
    }
}

/// 설정 파일 본문을 읽는다. 모르는 항목은 무시하고, 잘못된 값과 읽지 못한 줄은 `notes` 에 남기며 기본값을 쓴다.
pub fn parse_toml(text: &str) -> (Settings, Vec<String>) {
    let mut s = Settings::default();
    let mut notes = Vec::new();
    for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || (t.starts_with('[') && t.ends_with(']')) {
            continue;
        }
        let Some((key, val)) = t.split_once('=') else {
            notes.push(format!("{}번째 줄을 읽지 못했습니다: {t}", i + 1));
            continue;
        };
        let Some(v) = parse_value(val) else {
            notes.push(format!("{}번째 줄의 값을 읽지 못했습니다: {t}", i + 1));
            continue;
        };
        match key.trim() {
            "result_dir" | "output_dir" if !v.trim().is_empty() => s.result_dir = PathBuf::from(v),
            "download_dir" | "kakao_photo_dir" if !v.trim().is_empty() => s.download_dir = PathBuf::from(v),
            "videos" => match Videos::parse(v.trim()) {
                Some(m) => s.videos = m,
                None => notes.push(format!("videos 값 {v:?} 을(를) 알 수 없어 \"{}\" 를 씁니다 (ask, keep, skip 중 하나)", s.videos.as_str())),
            },
            _ => {}
        }
    }
    (s, notes)
}

/// 예전 `config.json` 형식을 읽는다.
fn parse_legacy_json(text: &str) -> Result<Settings, String> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if !v.is_object() {
        return Err("최상위가 객체({...})가 아니다".into());
    }
    let d = Settings::default();
    let path = |k: &str, dflt: PathBuf| v.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(PathBuf::from).unwrap_or(dflt);
    Ok(Settings {
        result_dir: path("output_dir", d.result_dir),
        download_dir: path("kakao_photo_dir", d.download_dir),
        videos: v.get("videos").and_then(Value::as_str).and_then(Videos::parse).unwrap_or(d.videos),
    })
}

impl Settings {
    pub fn load(path: &Path) -> Loaded {
        match fs::read_to_string(path) {
            Ok(text) => {
                let (settings, notes) = parse_toml(&text);
                Loaded { settings, status: LoadStatus::Loaded, notes }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let legacy = path.with_file_name("config.json");
                let mut notes = Vec::new();
                let mut status = LoadStatus::Created;
                let mut settings = Settings::default();
                if legacy.exists() {
                    match fs::read_to_string(&legacy).map_err(|e| e.to_string()).and_then(|t| parse_legacy_json(&t)) {
                        Ok(s) => {
                            settings = s;
                            status = LoadStatus::Migrated(legacy);
                        }
                        Err(why) => notes.push(format!("옛 설정 파일 {} 을(를) 읽지 못해 기본값을 씁니다 ({why})", legacy.display())),
                    }
                }
                match settings.save(path) {
                    Ok(()) => Loaded { settings, status, notes },
                    Err(e) => Loaded { settings, status: LoadStatus::Invalid(format!("설정 파일을 만들지 못했다: {e}")), notes },
                }
            }
            Err(e) => Loaded { settings: Settings::default(), status: LoadStatus::Invalid(e.to_string()), notes: Vec::new() },
        }
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
        assert_eq!(w, PathBuf::from(r"C:\Users\a\AppData\Roaming").join("kkt-manual-export").join("config.toml"));
        let w2 = config_path_from("windows", env(&[("USERPROFILE", r"C:\Users\a")]));
        assert!(w2.starts_with(PathBuf::from(r"C:\Users\a").join("AppData").join("Roaming")));
        let m = config_path_from("macos", env(&[("HOME", "/Users/a")]));
        assert_eq!(m, PathBuf::from("/Users/a/Library/Application Support/kkt-manual-export/config.toml"));
        let x = config_path_from("other", env(&[("HOME", "/home/a")]));
        assert_eq!(x, PathBuf::from("/home/a/.config/kkt-manual-export/config.toml"));
        let x2 = config_path_from("other", env(&[("HOME", "/home/a"), ("XDG_CONFIG_HOME", "/cfg")]));
        assert_eq!(x2, PathBuf::from("/cfg/kkt-manual-export/config.toml"));
        assert_eq!(config_path_from("windows", env(&[("KKT_CONFIG", "/tmp/my.toml"), ("APPDATA", "/x")])), PathBuf::from("/tmp/my.toml"));
    }

    #[test]
    fn written_file_explains_every_choice_and_round_trips() {
        let s = Settings { result_dir: PathBuf::from(r"C:\Users\a\Downloads\kkt"), download_dir: PathBuf::from(r"D:\카카오톡 받은 파일"), videos: Videos::Keep };
        let text = s.to_toml();
        assert!(text.contains("result_dir = 'C:\\Users\\a\\Downloads\\kkt'"), "경로는 작은따옴표 안에 그대로: {text}");
        for needle in ["\"ask\"", "\"keep\"", "\"skip\"", "# 결과 폴더", "# 다운로드 폴더", "videos = \"keep\""] {
            assert!(text.contains(needle), "{needle} 가 있어야 한다:\n{text}");
        }
        let (back, notes) = parse_toml(&text);
        assert_eq!(back, s);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn paths_with_apostrophes_are_escaped_and_read_back() {
        let s = Settings { result_dir: PathBuf::from("C:\\Users\\O'Neil\\x\"y"), download_dir: PathBuf::from("/tmp/a"), videos: Videos::Ask };
        let text = s.to_toml();
        assert!(text.contains("result_dir = \"C:\\\\Users\\\\O'Neil\\\\x\\\"y\""), "{text}");
        assert_eq!(parse_toml(&text).0, s);
    }

    #[test]
    fn hand_written_values_are_read_leniently() {
        let t = "\u{feff}# 주석\r\n[무시되는 구역]\r\nresult_dir = 'C:\\a b\\c'   # 뒤의 주석\r\ndownload_dir=\"C:\\\\x\"\r\nvideos = skip\r\nunknown = 1\r\n";
        let (s, notes) = parse_toml(t);
        assert_eq!(s.result_dir, PathBuf::from("C:\\a b\\c"));
        assert_eq!(s.download_dir, PathBuf::from("C:\\x"));
        assert_eq!(s.videos, Videos::Skip);
        assert!(notes.is_empty(), "{notes:?}");
        // 이중 따옴표 안의 모르는 이스케이프(\U)는 그대로 둔다 (\ 를 한 번만 쓴 경로)
        assert_eq!(parse_toml("result_dir = \"C:\\Users\\a\"").0.result_dir, PathBuf::from("C:\\Users\\a"));
        // 옛 이름도 읽는다
        assert_eq!(parse_toml("output_dir = '/o'\nkakao_photo_dir = '/k'").0, Settings { result_dir: "/o".into(), download_dir: "/k".into(), videos: Videos::Ask });
    }

    #[test]
    fn bad_lines_and_values_become_notes_and_defaults() {
        let (s, notes) = parse_toml("videos = \"maybe\"\nthis is not a setting\nresult_dir = 'unterminated\n");
        assert_eq!(s, Settings::default());
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes[0].contains("videos") && notes[0].contains("ask, keep, skip"));
        assert!(notes[1].contains("2번째 줄") && notes[2].contains("3번째 줄"));
        assert_eq!(parse_toml("result_dir = ''\n").0, Settings::default(), "빈 경로는 기본값");
    }

    #[test]
    fn videos_labels_cycle_and_parse() {
        assert_eq!(Videos::Ask.label(), "물어보기");
        assert_eq!(Videos::Keep.label(), "항상 보관");
        assert_eq!(Videos::Skip.label(), "보관 안 함");
        assert_eq!(Videos::Ask.next(), Videos::Keep);
        assert_eq!(Videos::Keep.next(), Videos::Skip);
        assert_eq!(Videos::Skip.next(), Videos::Ask);
        for v in Videos::ALL {
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
        let path = dir.join("sub").join("config.toml");
        let l = Settings::load(&path);
        assert_eq!(l.status, LoadStatus::Created);
        assert!(path.exists() && l.notes.is_empty());
        let mut changed = l.settings.clone();
        changed.videos = Videos::Skip;
        changed.result_dir = PathBuf::from("/somewhere/else");
        changed.save(&path).unwrap();
        let again = Settings::load(&path);
        assert_eq!(again.status, LoadStatus::Loaded);
        assert_eq!(again.settings, changed);
        assert!(!path.with_extension("toml.tmp").exists(), "임시 파일은 남지 않는다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_edits_survive_and_comments_come_back_on_save() {
        let dir = tmp("edit");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "result_dir = '/my/out'\n").unwrap(); // 주석 없이 직접 쓴 파일
        let l = Settings::load(&path);
        assert_eq!(l.settings.result_dir, PathBuf::from("/my/out"));
        l.settings.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("'/my/out'") && text.contains("\"skip\""), "저장하면 선택지 주석이 다시 생긴다:\n{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_json_is_migrated_and_kept() {
        let dir = tmp("legacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            "{\n  \"kakao_photo_dir\": \"C:\\\\Users\\\\a\\\\Documents\\\\카카오톡 받은 파일\",\n  \"output_dir\": \"C:\\\\out\",\n  \"videos\": \"keep\"\n}\n",
        )
        .unwrap();
        let path = dir.join("config.toml");
        let l = Settings::load(&path);
        assert!(matches!(l.status, LoadStatus::Migrated(_)));
        assert_eq!(l.settings.result_dir, PathBuf::from("C:\\out"));
        assert_eq!(l.settings.download_dir, PathBuf::from("C:\\Users\\a\\Documents\\카카오톡 받은 파일"));
        assert_eq!(l.settings.videos, Videos::Keep);
        assert!(path.exists() && dir.join("config.json").exists(), "새 파일을 만들고 옛 파일은 그대로 둔다");
        // 이제 toml 만 읽는다
        fs::write(dir.join("config.json"), "{ 망가짐").unwrap();
        assert_eq!(Settings::load(&path).status, LoadStatus::Loaded);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_legacy_json_falls_back_to_defaults_with_a_note() {
        let dir = tmp("legacybad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{ not json").unwrap();
        let l = Settings::load(&dir.join("config.toml"));
        assert_eq!(l.status, LoadStatus::Created);
        assert_eq!(l.settings, Settings::default());
        assert!(l.notes.iter().any(|n| n.contains("config.json")), "{:?}", l.notes);
        assert_eq!(fs::read_to_string(dir.join("config.json")).unwrap(), "{ not json", "옛 파일은 건드리지 않는다");
        let _ = fs::remove_dir_all(&dir);
    }
}
