//! Python 과 같은 출력을 내기 위한 서식 도구.
//!
//! - `dumps`: `json.dumps(v, ensure_ascii=False, sort_keys=True)` 와 같은 바이트 (구분자 ", " 와 ": ").
//!   serde_json 의 Map 은 BTreeMap 이라 키가 정렬된다. 한글은 이스케이프하지 않는다.
//! - `repr`: 경고 문구에 쓰이는 `{x!r}` (str 의 repr).
//! - `strip`: `str.strip()` (유니코드 공백 + \x1c-\x1f).

use serde::Serialize;
use serde_json::ser::Formatter;
use serde_json::Value;
use std::io;

struct PyFormatter;

impl Formatter for PyFormatter {
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }
    fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }
}

pub fn dumps(v: &Value) -> String {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, PyFormatter);
    v.serialize(&mut ser).expect("JSON 직렬화는 실패하지 않는다");
    String::from_utf8(buf).expect("serde_json 은 항상 UTF-8 을 낸다")
}

fn is_printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    if c.is_whitespace() || c.is_control() {
        return false;
    }
    !matches!(c as u32,
        0xAD | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0xFEFF | 0xE000..=0xF8FF
        | 0xFFF9..=0xFFFB | 0xD800..=0xDFFF)
}

/// Python `repr(str)`. 따옴표 선택과 이스케이프 규칙을 따른다.
/// (출력 가능 문자 판정은 근사다: 유니코드 범주표가 없어서 흔한 비출력 문자만 이스케이프한다.)
pub fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if !is_printable(c) => {
                let n = c as u32;
                if n < 0x100 {
                    out.push_str(&format!("\\x{n:02x}"));
                } else if n < 0x10000 {
                    out.push_str(&format!("\\u{n:04x}"));
                } else {
                    out.push_str(&format!("\\U{n:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `{x!r}` 로 None 을 찍으면 `None` 이다.
pub fn repr_opt(s: Option<&str>) -> String {
    match s {
        Some(s) => repr(s),
        None => "None".to_string(),
    }
}

fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `str.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dumps_matches_python_separators_and_sorting() {
        let v = json!({"b": [1, 2, {"z": null, "a": "한글"}], "a": true, "c": "q\"\\\n\u{1}\u{7f}"});
        assert_eq!(dumps(&v), r#"{"a": true, "b": [1, 2, {"a": "한글", "z": null}], "c": "q\"\\\n\u0001"#.to_string() + "\u{7f}\"}");
    }

    #[test]
    fn dumps_empty_containers() {
        assert_eq!(dumps(&json!({"a": [], "b": {}})), r#"{"a": [], "b": {}}"#);
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr("철수"), "'철수'");
        assert_eq!(repr("it's"), "\"it's\"");
        assert_eq!(repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(repr("a\nb\t\\"), "'a\\nb\\t\\\\'");
        assert_eq!(repr("\u{1}x"), "'\\x01x'");
        assert_eq!(repr_opt(None), "None");
    }

    #[test]
    fn strip_matches_python() {
        assert_eq!(strip("  a b \u{3000}\u{1c}"), "a b");
        assert_eq!(strip(""), "");
    }
}
