//! 카카오톡 PC '대화 내보내기' TXT 파서 (src/python/kkt/parse.py 의 이식).
//!
//! 관측된 형식:
//!
//! ```text
//! test 님과 카카오톡 대화
//! 저장한 날짜 : 2026-10-02 10:18:07
//!
//! --------------- 2026년 10월 2일 금요일 ---------------
//! .님이 A님, B님을 초대했습니다.            <- 시스템 이벤트
//! [.] [오전 9:27] Test                      <- 메시지
//! [.] [오전 9:28] 사진                      <- 사진은 '사진' 한 줄 (한 번에 여러 장이면 '사진 16장' 한 줄)
//! 메시지가 삭제되었습니다.                  <- '모두에게 삭제'. 보낸이/시각 없음
//! ```
//!
//! 줄 구분은 `\r\n`, `\n`, `\r` 뿐이다 (Python `splitlines()` 의 U+2028, U+0085 등은 쓰지 않는다).

use crate::pyfmt::{repr, strip};
use regex_lite::Regex;
use std::sync::OnceLock;

pub const DELETED_MARKER: &str = "메시지가 삭제되었습니다.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Message,
    DeletedMarker,
    System,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Message => "message",
            Kind::DeletedMarker => "deleted_marker",
            Kind::System => "system",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    pub line_no: usize,
    pub date: String,
    pub hhmm: Option<String>,
    pub sender: Option<String>,
    pub text: String,
    pub content_type: String,
    /// content_type 이 image 일 때 사진 수 ('사진 16장' 이면 16)
    pub image_count: usize,
    pub raw_lines: Vec<String>,
}

#[derive(Debug)]
pub struct ParsedExport {
    pub title: Option<String>,
    pub saved_at: Option<String>,
    pub entries: Vec<Entry>,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub warnings: Vec<String>,
}

struct Res {
    title: Regex,
    saved: Regex,
    date: Regex,
    msg: Regex,
    system: Vec<Regex>,
}

fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| Res {
        title: Regex::new(r"^(.*) 님과 카카오톡 대화$").unwrap(),
        saved: Regex::new(r"^저장한 날짜 : (\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})$").unwrap(),
        date: Regex::new(r"^-{3,} (\d{4})년 (\d{1,2})월 (\d{1,2})일 \S*요일 -{3,}$").unwrap(),
        msg: Regex::new(r"^\[(.+?)\] \[(오전|오후) (\d{1,2}):(\d{2})\] (.*)$").unwrap(),
        // 관측된 시스템 문구만 등록한다. 여기 없는 줄은 앞 메시지의 이어지는 줄로 본다.
        system: vec![
            Regex::new(r"^.+님이 .+님을 초대했습니다\.$").unwrap(),
            Regex::new(r"^.+님이 들어왔습니다\.$").unwrap(),
            Regex::new(r"^.+님이 나갔습니다\.$").unwrap(),
        ],
    })
}

pub fn to_24h(ampm: &str, hour: u32) -> u32 {
    if ampm == "오전" {
        if hour == 12 { 0 } else { hour }
    } else if hour == 12 {
        12
    } else {
        hour + 12
    }
}

/// 줄 구분은 \r\n, \n, \r 만이다. 마지막 빈 조각은 버린다 (Python 구현의 `split_lines` 와 같다).
pub fn split_lines(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                parts.push(std::mem::take(&mut cur));
            }
            '\n' => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    if parts.last().map(|s| s.is_empty()).unwrap_or(false) {
        parts.pop();
    }
    parts
}

/// "사진" 은 1장, "사진 16장" 은 16장 (한 번에 여러 장 보낸 줄, 실측). 그 밖에는 사진이 아니다.
/// 글자 그대로 "사진 2장" 이라고 보낸 메시지와는 TXT 만으로 구분할 수 없다. 숫자는 ASCII 만 받는다.
fn image_count(text: &str) -> Option<usize> {
    if text == "사진" {
        return Some(1);
    }
    let digits = text.strip_prefix("사진 ")?.strip_suffix('장')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<usize>().ok().filter(|&n| n >= 1)
}

fn close(cur: &mut Option<Entry>, entries: &mut Vec<Entry>) {
    if let Some(mut e) = cur.take() {
        while e.raw_lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            e.raw_lines.pop();
        }
        e.text = e.raw_lines.join("\n");
        if e.kind == Kind::Message {
            if let Some(n) = image_count(&e.text) {
                e.content_type = "image".to_string();
                e.image_count = n;
            } else if e.text == "이모티콘" {
                e.content_type = "emoticon".to_string();
            } else if e.text == "동영상" {
                e.content_type = "video".to_string(); // 실측: 동영상은 '동영상' 한 줄이다
            }
        }
        entries.push(e);
    }
}

pub fn parse_export(text: &str) -> ParsedExport {
    let r = res();
    let text = text.trim_start_matches('\u{feff}');
    let lines = split_lines(text);
    let mut title: Option<String> = None;
    let mut saved_at: Option<String> = None;
    let mut entries: Vec<Entry> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut cur_date: Option<String> = None;
    let mut cur: Option<Entry> = None;

    for (i0, raw) in lines.iter().enumerate() {
        let i = i0 + 1;
        let line: &str = raw.trim_end_matches('\r');

        if i <= 3 {
            if title.is_none() {
                if let Some(m) = r.title.captures(line) {
                    title = Some(m[1].to_string());
                    continue;
                }
            }
            if saved_at.is_none() {
                if let Some(m) = r.saved.captures(line) {
                    saved_at = Some(format!("{}-{}-{}T{}:{}:{}+09:00", &m[1], &m[2], &m[3], &m[4], &m[5], &m[6]));
                    continue;
                }
            }
        }

        if let Some(m) = r.date.captures(line) {
            close(&mut cur, &mut entries);
            let (y, mo, d): (u32, u32, u32) = (m[1].parse().unwrap(), m[2].parse().unwrap(), m[3].parse().unwrap());
            cur_date = Some(format!("{y:04}-{mo:02}-{d:02}"));
            continue;
        }

        let date = match &cur_date {
            None => continue, // 첫 날짜 헤더 전의 줄(헤더, 빈 줄)은 무시
            Some(d) => d.clone(),
        };

        if let Some(m) = r.msg.captures(line) {
            close(&mut cur, &mut entries);
            let hh = to_24h(&m[2], m[3].parse().unwrap());
            cur = Some(Entry {
                kind: Kind::Message,
                line_no: i,
                date,
                hhmm: Some(format!("{hh:02}:{}", &m[4])),
                sender: Some(m[1].to_string()),
                text: String::new(),
                content_type: "text".to_string(),
                image_count: 1,
                raw_lines: vec![m[5].to_string()],
            });
            continue;
        }

        if line == DELETED_MARKER {
            close(&mut cur, &mut entries);
            entries.push(Entry {
                kind: Kind::DeletedMarker,
                line_no: i,
                date,
                hhmm: None,
                sender: None,
                text: line.to_string(),
                content_type: "text".to_string(),
                image_count: 1,
                raw_lines: vec![line.to_string()],
            });
            continue;
        }

        if r.system.iter().any(|re| re.is_match(line)) {
            close(&mut cur, &mut entries);
            entries.push(Entry {
                kind: Kind::System,
                line_no: i,
                date,
                hhmm: None,
                sender: None,
                text: line.to_string(),
                content_type: "text".to_string(),
                image_count: 1,
                raw_lines: vec![line.to_string()],
            });
            continue;
        }

        // 그 외: 앞 메시지의 이어지는 줄 (빈 줄 포함)
        if let Some(c) = cur.as_mut() {
            c.raw_lines.push(line.to_string());
        } else if !strip(line).is_empty() {
            let head: String = line.chars().take(40).collect();
            warnings.push(format!("line {i}: 해석할 수 없는 줄 (무시): {}", repr(&head)));
        }
    }
    close(&mut cur, &mut entries);

    let first_date = entries.iter().map(|e| e.date.clone()).min();
    let last_date = entries.iter().map(|e| e.date.clone()).max();
    if title.is_none() {
        warnings.push("제목 줄을 찾지 못했다".to_string());
    }
    if saved_at.is_none() {
        warnings.push("'저장한 날짜' 줄을 찾지 못했다".to_string());
    }
    ParsedExport { title, saved_at, entries, first_date, last_date, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(lines: &[&str]) -> String {
        let mut v = vec!["room 님과 카카오톡 대화", "저장한 날짜 : 2026-10-02 10:18:07", "", "--------------- 2026년 10월 2일 금요일 ---------------"];
        v.extend_from_slice(lines);
        v.join("\r\n") + "\r\n"
    }

    #[test]
    fn split_lines_only_uses_cr_lf() {
        assert_eq!(split_lines("a\r\nb\nc\rd\r\n"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines("a\u{2028}b\u{85}c\x0bd"), vec!["a\u{2028}b\u{85}c\x0bd"]);
        assert_eq!(split_lines(""), Vec::<String>::new());
        assert_eq!(split_lines("\n"), vec![""]);
        assert_eq!(split_lines("a\n\nb"), vec!["a", "", "b"]);
    }

    #[test]
    fn kinds_and_times() {
        let p = parse_export(&export(&[
            ".님이 A님, B님을 초대했습니다.",
            "[me] [오전 9:27] hello",
            "[me] [오전 9:28] 사진",
            "메시지가 삭제되었습니다.",
            "[bob] [오후 12:05] noon",
            "[bob] [오전 12:05] midnight",
            "[bob] [오전 9:30] 이모티콘",
        ]));
        assert_eq!(p.title.as_deref(), Some("room"));
        assert_eq!(p.saved_at.as_deref(), Some("2026-10-02T10:18:07+09:00"));
        let kinds: Vec<_> = p.entries.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["system", "message", "message", "deleted_marker", "message", "message", "message"]);
        assert_eq!(p.entries[1].hhmm.as_deref(), Some("09:27"));
        assert_eq!(p.entries[2].content_type, "image");
        assert_eq!(p.entries[4].hhmm.as_deref(), Some("12:05"));
        assert_eq!(p.entries[5].hhmm.as_deref(), Some("00:05"));
        assert_eq!(p.entries[6].content_type, "emoticon");
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn multiline_and_bom() {
        let p = parse_export(&format!("\u{feff}{}", export(&["[me] [오전 9:00] l1", "l2", "", "l4", "[me] [오전 9:01] next"])));
        assert_eq!(p.entries[0].text, "l1\nl2\n\nl4");
        assert_eq!(p.entries[1].text, "next");
    }

    #[test]
    fn image_count_lines() {
        assert_eq!(image_count("사진"), Some(1));
        assert_eq!(image_count("사진 16장"), Some(16));
        assert_eq!(image_count("사진 1장"), Some(1));
        for text in ["사진 0장", "사진 2장 찍었어", "사진 장", "사진  2장", "사진 ２장", "사진 -1장", "사진2장", "사진찍었어"] {
            assert_eq!(image_count(text), None, "{text}");
        }
    }
}
