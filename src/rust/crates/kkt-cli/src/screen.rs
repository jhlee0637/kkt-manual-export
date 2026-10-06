//! 화면 그리기: 상자, 글자 폭 계산.
//!
//! 한글·한자·전각 문자는 터미널에서 두 칸을 차지한다. 오른쪽 선을 맞추려면 글자 수가 아니라 칸 수로 센다.
//! 상자 선(`┌ ─ ┐ │ └ ┘`)은 한 칸이다. 터미널이 상자보다 좁으면 선 없이 줄만 보여 준다 (경로를 줄이지 않기 위해서다:
//! 줄인 경로는 복사해서 쓸 수 없다).

/// 문자 하나가 차지하는 칸 수 (동아시아 전각 문자는 2).
fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide = matches!(u,
        0x1100..=0x115F      // 한글 자모
        | 0x2E80..=0xA4CF    // CJK 부수, 한자, 가나 등
        | 0xAC00..=0xD7A3    // 한글 음절
        | 0xF900..=0xFAFF    // CJK 호환 한자
        | 0xFE30..=0xFE6F    // CJK 호환 형태
        | 0xFF00..=0xFF60    // 전각 형태
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1FAFF  // 이모지
        | 0x20000..=0x3FFFD  // CJK 확장
    );
    if wide { 2 } else { 1 }
}

/// 문자열이 터미널에서 차지하는 칸 수.
pub fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// 상자 안의 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// 미리 들여쓴 본문 (왼쪽 들여쓰기는 호출한 쪽이 넣는다)
    Text(String),
    Blank,
    /// 가로 구분선
    Rule,
}

/// 칸 수 기준으로 오른쪽을 공백으로 채운다.
fn pad_to(s: &str, width: usize) -> String {
    let w = display_width(s);
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

/// 상자 안쪽 폭(칸): 가장 긴 줄 + 오른쪽 여백 2, 제목이 더 길면 제목에 맞춘다.
pub fn inner_width(title: &str, rows: &[Row]) -> usize {
    let body = rows.iter().filter_map(|r| if let Row::Text(t) = r { Some(display_width(t)) } else { None }).max().unwrap_or(0);
    (body + 2).max(display_width(title) + 4)
}

/// 상자로 그린 줄들.
pub fn render_box(title: &str, rows: &[Row]) -> Vec<String> {
    let inner = inner_width(title, rows);
    let mut out = Vec::with_capacity(rows.len() + 2);
    out.push(format!("┌─ {title} {}┐", "─".repeat(inner - display_width(title) - 3)));
    for r in rows {
        let line = match r {
            Row::Text(t) => pad_to(t, inner),
            Row::Blank => " ".repeat(inner),
            Row::Rule => format!("  {}  ", "─".repeat(inner.saturating_sub(4))),
        };
        out.push(format!("│{line}│"));
    }
    out.push(format!("└{}┘", "─".repeat(inner)));
    out
}

/// 상자 없이 줄만 (터미널이 좁을 때). 구분선은 짧게 그린다.
pub fn render_plain(title: &str, rows: &[Row]) -> Vec<String> {
    let mut out = vec![format!("== {title} ==")];
    for r in rows {
        out.push(match r {
            Row::Text(t) => t.clone(),
            Row::Blank => String::new(),
            Row::Rule => "  ----".to_string(),
        });
    }
    out
}

/// 터미널 폭(칸). 환경변수 `COLUMNS` 가 있으면 그것, 없으면 Windows 콘솔의 창 폭. 알 수 없으면 `None`.
pub fn terminal_width() -> Option<usize> {
    std::env::var("COLUMNS").ok().and_then(|v| v.trim().parse().ok()).filter(|w| *w > 0).or_else(kkt_win::sys::console_width)
}

/// 상자가 터미널에 들어가면 상자로, 아니면 줄만 그린다. 폭을 알 수 없으면 상자로 그린다.
pub fn render(title: &str, rows: &[Row], term_width: Option<usize>) -> Vec<String> {
    let boxed = render_box(title, rows);
    match term_width {
        Some(w) if boxed.iter().map(|l| display_width(l)).max().unwrap_or(0) > w => render_plain(title, rows),
        _ => boxed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Row {
        Row::Text(s.to_string())
    }

    #[test]
    fn hangul_counts_two_columns_and_box_lines_one() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("저장 폴더"), 9);
        assert_eq!(display_width("C:\\카카오톡"), 2 + 1 + 8);
        assert_eq!(display_width("├─ archive\\"), 11);
    }

    #[test]
    fn every_box_line_has_the_same_width() {
        let rows = [Row::Blank, t("  결과 폴더    C:\\Users\\spgmb\\Downloads"), Row::Rule, t("  동영상 보관    [물어보기]  항상 보관  보관 안 함"), Row::Blank];
        let b = render_box("kkt-manual-export-v0.3.0", &rows);
        let widths: Vec<usize> = b.iter().map(|l| display_width(l)).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}\n{}", b.join("\n"));
        assert!(b[0].starts_with("┌─ kkt-manual-export-v0.3.0 ─") && b[0].ends_with('┐'));
        assert!(b.last().unwrap().starts_with('└') && b.last().unwrap().ends_with('┘'));
    }

    #[test]
    fn the_box_grows_to_fit_the_longest_path_without_cutting_it() {
        let long = format!("  {}", "C:\\".to_string() + &"긴폴더\\".repeat(20));
        let b = render_box("제목", &[t(&long), t("  짧음")]);
        assert!(b.iter().any(|l| l.contains(long.trim())), "경로는 그대로 들어간다");
        let widths: Vec<usize> = b.iter().map(|l| display_width(l)).collect();
        assert!(widths.iter().all(|w| *w == widths[0]));
    }

    #[test]
    fn rule_leaves_two_columns_of_margin_on_each_side() {
        let b = render_box("t", &[t("  0123456789"), Row::Rule]);
        let inner = inner_width("t", &[t("  0123456789"), Row::Rule]);
        assert_eq!(b[2], format!("│  {}  │", "─".repeat(inner - 4)));
    }

    #[test]
    fn narrow_terminals_get_plain_lines_not_a_broken_box() {
        let rows = [t("  C:\\Users\\spgmb\\Downloads\\kkt-manual-export-archive"), Row::Rule];
        let wide = render("제목", &rows, Some(200));
        assert!(wide[0].starts_with('┌'));
        let unknown = render("제목", &rows, None);
        assert!(unknown[0].starts_with('┌'), "폭을 모르면 상자로 그린다");
        let narrow = render("제목", &rows, Some(30));
        assert_eq!(narrow[0], "== 제목 ==");
        assert_eq!(narrow[1], "  C:\\Users\\spgmb\\Downloads\\kkt-manual-export-archive", "경로는 줄이지 않는다");
        assert!(narrow.iter().all(|l| !l.contains('│')));
    }

    #[test]
    fn title_wider_than_the_body_widens_the_box() {
        let b = render_box("아주 긴 제목이 본문보다 넓은 경우", &[t("  a")]);
        let widths: Vec<usize> = b.iter().map(|l| display_width(l)).collect();
        assert!(widths.iter().all(|w| *w == widths[0]) && widths[0] >= display_width("아주 긴 제목이 본문보다 넓은 경우") + 6);
    }
}
