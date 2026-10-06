//! 채팅방 서랍의 사진 격자를 캡처 이미지에서 읽는다 (순수 로직, OS 무관).
//!
//! 실측(서랍 창 840x600): 썸네일은 가로 3개, 칸 124px, 간격 132px. 왼쪽 끝은 창 왼쪽에서 284px.
//! 첫 줄은 탭 구분선(y≈175)과 월 머리글(y≈203) 아래 y=225 에서 시작한다.
//! 썸네일에 마우스를 올리면 왼쪽 위(칸 기준 (15,16))에 선택 원이 생기고, 선택하면 노란 체크가 된다.
//! 화면의 글자는 읽을 수 없으므로(직접 그리는 컨트롤) 개수와 선택 여부는 픽셀로 판단한다.

use crate::sys::Image;

pub const GRID_LEFT: i32 = 284;
pub const PITCH: i32 = 132;
pub const CELL: i32 = 124;
pub const COLS: i32 = 3;
/// 이 아래부터가 격자 영역 (탭 구분선 아래).
pub const TOP_MIN: i32 = 180;
/// 선택 바(`N 개 선택`)가 차지하는 아래쪽 높이. 칸이 이 안에 걸치면 쓰지 않는다.
pub const BOTTOM_RESERVED: i32 = 62;
/// 칸 왼쪽 위에서 선택 원 중심까지.
pub const CIRCLE_DX: i32 = 15;
pub const CIRCLE_DY: i32 = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// 창 안에서의 왼쪽 위
    pub x: i32,
    pub y: i32,
    /// 사진 내용에서 만든 서명. 스크롤해도 같은 사진이면 같다.
    pub sig: u64,
    pub selected: bool,
}

impl Cell {
    pub fn circle(&self) -> (i32, i32) {
        (self.x + CIRCLE_DX, self.y + CIRCLE_DY)
    }
}

fn non_white(p: (u8, u8, u8)) -> bool {
    !(p.0 >= 242 && p.1 >= 242 && p.2 >= 242)
}

fn is_yellow(p: (u8, u8, u8)) -> bool {
    p.0 > 235 && p.1 > 190 && p.2 < 90
}

/// 줄 `y` 에 격자 폭 안에서 흰색이 아닌 점이 충분히 있는가.
fn row_has_content(img: &Image, y: i32) -> bool {
    let (x0, x1) = (GRID_LEFT, GRID_LEFT + COLS * PITCH - (PITCH - CELL));
    let mut hit = 0;
    let mut total = 0;
    let mut x = x0;
    while x < x1 && (x as usize) < img.w {
        total += 1;
        if non_white(img.rgb(x as usize, y as usize)) {
            hit += 1;
        }
        x += 4;
    }
    total > 0 && hit * 100 >= total * 4
}

fn cell_present(img: &Image, x0: i32, y0: i32) -> bool {
    let mut hit = 0;
    let mut total = 0;
    let mut y = y0 + 6;
    while y < y0 + CELL - 6 {
        let mut x = x0 + 6;
        while x < x0 + CELL - 6 {
            if (x as usize) < img.w && (y as usize) < img.h {
                total += 1;
                if non_white(img.rgb(x as usize, y as usize)) {
                    hit += 1;
                }
            }
            x += 6;
        }
        y += 6;
    }
    total > 0 && hit * 100 >= total * 10
}

fn signature(img: &Image, x0: i32, y0: i32) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut y = y0 + 8;
    while y < y0 + CELL - 8 {
        let mut x = x0 + 8;
        while x < x0 + CELL - 8 {
            // 왼쪽 위(선택 원이 생기는 곳)는 빼서 마우스를 올려도 서명이 같게 한다
            if !(x < x0 + 46 && y < y0 + 46) {
                let (r, g, b) = img.rgb(x as usize, y as usize);
                for v in [r >> 4, g >> 4, b >> 4] {
                    h ^= v as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
            x += 6;
        }
        y += 6;
    }
    h
}

fn is_selected(img: &Image, cx: i32, cy: i32) -> bool {
    for dy in -6..=6 {
        for dx in -6..=6 {
            let (x, y) = (cx + dx, cy + dy);
            if x >= 0 && y >= 0 && (x as usize) < img.w && (y as usize) < img.h && is_yellow(img.rgb(x as usize, y as usize)) {
                return true;
            }
        }
    }
    false
}

/// 영역 안에서 조건에 맞는 픽셀 수.
fn count_region(img: &Image, x0: i32, y0: i32, w: i32, h: i32, f: impl Fn((u8, u8, u8)) -> bool) -> usize {
    let mut n = 0;
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            if x >= 0 && y >= 0 && (x as usize) < img.w && (y as usize) < img.h && f(img.rgb(x as usize, y as usize)) {
                n += 1;
            }
        }
    }
    n
}

/// 동영상 타일인가. 동영상은 오른쪽 아래에 어두운 알약과 그 안의 흰 재생 시간(`00:04`)이 있다.
/// 묶음(`사진 N장`) 타일은 같은 자리에 작은 어두운 정사각형 아이콘(흰 그림 픽셀이 많다)이 있어서 구별된다.
///
/// 실측(타일 11개: 동영상 1, 묶음 3, 사진 7)으로 기준을 잡았다.
/// 알약의 왼쪽(타일 안 x 86..102)은 어둡고 흰 글자 획이 조금 있으며, 오른쪽(x 104..120)도 같다.
/// 오른쪽의 흰 픽셀은 동영상 18개, 묶음 아이콘 42개 이상이다.
pub fn is_video_tile(img: &Image, c: &Cell) -> bool {
    let dark = |p: (u8, u8, u8)| p.0.max(p.1).max(p.2) < 95;
    let white = |p: (u8, u8, u8)| p.0.min(p.1).min(p.2) >= 200;
    let (ld, lw) = (count_region(img, c.x + 86, c.y + 100, 16, 16, dark), count_region(img, c.x + 86, c.y + 100, 16, 16, white));
    let (rd, rw) = (count_region(img, c.x + 104, c.y + 100, 16, 18, dark), count_region(img, c.x + 104, c.y + 100, 16, 18, white));
    ld >= 90 && (8..=40).contains(&lw) && rd >= 90 && (8..=34).contains(&rw)
}

/// 이미지에서 완전히 보이는 썸네일 칸을 위→아래, 왼쪽→오른쪽 순서로 찾는다.
pub fn find_cells(img: &Image) -> Vec<Cell> {
    // 선택 바는 칸을 하나라도 고른 뒤에만 생긴다. 바가 없으면 마지막 줄이 창 아래까지 내려오므로 자리를 비우지 않는다 (실측).
    let y_end = if selection_bar_visible(img) { img.h as i32 - BOTTOM_RESERVED } else { img.h as i32 - 4 };
    // 내용이 있는 줄의 연속 구간
    let mut runs: Vec<(i32, i32)> = Vec::new();
    let mut start: Option<i32> = None;
    for y in TOP_MIN..y_end.max(TOP_MIN) {
        let has = row_has_content(img, y);
        match (has, start) {
            (true, None) => start = Some(y),
            (false, Some(s)) => {
                runs.push((s, y - s));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, y_end - s)); // 아래가 잘린 구간: 길이 조건에서 걸러진다
    }
    let mut cells = Vec::new();
    for (y0, len) in runs {
        // 위가 잘린 구간(TOP_MIN 에서 시작)과 아래가 잘린 구간, 머리글 글자 줄은 칸이 아니다
        if y0 <= TOP_MIN || !(CELL - 6..=CELL + 8).contains(&len) {
            continue;
        }
        for c in 0..COLS {
            let x0 = GRID_LEFT + c * PITCH;
            if x0 + CELL > img.w as i32 || !cell_present(img, x0, y0) {
                continue;
            }
            let (cx, cy) = (x0 + CIRCLE_DX, y0 + CIRCLE_DY);
            cells.push(Cell { x: x0, y: y0, sig: signature(img, x0, y0), selected: is_selected(img, cx, cy) });
        }
    }
    cells
}

/// `저장 결과` 팝업(300x200)이 아직 저장 중인가. 저장 중에는 `파일 저장` 제목과 노란 진행 막대, `취소` 버튼이 보인다.
/// 이때 `Esc` 를 누르면 저장이 취소되므로 결과 팝업(`폴더 열기`)으로 바뀐 뒤에만 닫아야 한다 (실측).
pub fn save_in_progress(img: &Image) -> bool {
    let mut yellow = 0;
    for y in (img.h * 55 / 100)..(img.h * 70 / 100) {
        for x in 0..img.w {
            if is_yellow(img.rgb(x, y)) {
                yellow += 1;
            }
        }
    }
    yellow >= 60
}

/// 선택 바의 다운로드 아이콘 위치 (창 안 좌표). 실측: (너비−40, 높이−30).
pub fn download_icon(img: &Image) -> (i32, i32) {
    (img.w as i32 - 40, img.h as i32 - 30)
}

/// 선택 바가 보이는가: 다운로드 아이콘 자리는 짙고, 그 왼쪽(전달) 아이콘 자리에도 아이콘이 있다
/// (레이아웃이 달라졌는지 확인하는 안전장치). 영상이 선택에 섞이면 전달 아이콘이 회색으로 비활성이 되므로(실측)
/// 전달 아이콘은 짙지 않아도 "무언가 그려져 있음"만 본다.
pub fn selection_bar_visible(img: &Image) -> bool {
    let count = |cx: i32, cy: i32, limit: u8| -> usize {
        let mut n = 0;
        for dy in -10..=10 {
            for dx in -10..=10 {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as usize) < img.w && (y as usize) < img.h {
                    let (r, g, b) = img.rgb(x as usize, y as usize);
                    if r < limit && g < limit && b < limit {
                        n += 1;
                    }
                }
            }
        }
        n
    };
    let (dx, dy) = download_icon(img);
    count(dx, dy, 110) >= 8 && count(dx - 40, dy, 215) >= 8
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn blank(w: usize, h: usize) -> Image {
        Image { w, h, px: vec![255; w * h * 4] }
    }

    pub fn fill(img: &mut Image, x0: i32, y0: i32, w: i32, h: i32, rgb: (u8, u8, u8)) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let i = (y as usize * img.w + x as usize) * 4;
                img.px[i] = rgb.2;
                img.px[i + 1] = rgb.1;
                img.px[i + 2] = rgb.0;
            }
        }
    }

    /// 사진 칸처럼 보이는 가짜 썸네일 (seed 마다 다른 무늬)
    pub fn thumb(img: &mut Image, x0: i32, y0: i32, seed: u8) {
        fill(img, x0, y0, CELL, CELL, (120, 120, 120));
        for k in 0..8 {
            fill(img, x0 + 10 + k * 12, y0 + 10, 8, 100, (seed.wrapping_mul(k as u8 + 3), 40 + seed, 200 - seed));
        }
    }

    #[test]
    fn finds_two_cells_in_one_row() {
        let mut img = blank(840, 600);
        thumb(&mut img, GRID_LEFT, 225, 5);
        thumb(&mut img, GRID_LEFT + PITCH, 225, 9);
        fill(&mut img, 284, 197, 40, 12, (60, 60, 60)); // 월 머리글 글자 (칸이 아니다)
        let cells = find_cells(&img);
        assert_eq!(cells.len(), 2);
        assert_eq!((cells[0].x, cells[0].y), (284, 225));
        assert_eq!((cells[1].x, cells[1].y), (416, 225));
        assert_ne!(cells[0].sig, cells[1].sig);
        assert!(!cells[0].selected && !cells[1].selected);
    }

    #[test]
    fn detects_selected_badge_and_keeps_signature() {
        let mut img = blank(840, 600);
        thumb(&mut img, GRID_LEFT, 225, 5);
        let before = find_cells(&img)[0].clone();
        fill(&mut img, GRID_LEFT + 9, 225 + 10, 12, 12, (255, 215, 0)); // 노란 체크 배지
        let after = find_cells(&img)[0].clone();
        assert!(!before.selected && after.selected);
        assert_eq!(before.sig, after.sig, "선택해도 같은 사진의 서명은 같아야 한다");
        assert_eq!(after.circle(), (299, 241));
    }

    #[test]
    fn multiple_rows_and_columns_in_reading_order() {
        let mut img = blank(840, 600);
        for (r, y) in [225, 357].iter().enumerate() {
            for c in 0..3 {
                thumb(&mut img, GRID_LEFT + c * PITCH, *y, (r * 3 + c as usize + 1) as u8 * 7);
            }
        }
        let cells = find_cells(&img);
        assert_eq!(cells.len(), 6);
        let pos: Vec<_> = cells.iter().map(|c| (c.x, c.y)).collect();
        assert_eq!(pos, [(284, 225), (416, 225), (548, 225), (284, 357), (416, 357), (548, 357)]);
    }

    #[test]
    fn month_header_between_rows_shifts_next_row() {
        let mut img = blank(840, 700);
        thumb(&mut img, GRID_LEFT, 225, 5);
        fill(&mut img, 284, 380, 40, 12, (60, 60, 60)); // 다음 달 머리글
        thumb(&mut img, GRID_LEFT, 410, 11);
        let cells = find_cells(&img);
        assert_eq!(cells.iter().map(|c| c.y).collect::<Vec<_>>(), [225, 410]);
    }

    #[test]
    fn partially_visible_rows_are_ignored() {
        let mut img = blank(840, 600);
        thumb(&mut img, GRID_LEFT, 150, 5); // 위가 잘림 (TOP_MIN 위로 걸침)
        thumb(&mut img, GRID_LEFT, 470, 9); // 선택 바 자리(h-62)에 걸침
        fill(&mut img, 800 - 6, 570 - 6, 12, 12, (30, 30, 30)); // 선택 바가 보인다
        fill(&mut img, 760 - 6, 570 - 6, 12, 12, (30, 30, 30));
        assert!(find_cells(&img).is_empty());
    }

    /// 동영상 알약: 어두운 바탕에 흰 글자 획 (왼쪽·오른쪽 영역에 각각 24픽셀)
    fn video_pill(img: &mut Image, x0: i32, y0: i32) {
        fill(img, x0 + 84, y0 + 101, 34, 14, (20, 20, 20));
        for sx in [88, 94, 106, 112] {
            fill(img, x0 + sx, y0 + 103, 2, 6, (250, 250, 250));
        }
    }

    /// 묶음 아이콘: 어두운 정사각형 안에 흰 그림이 많다
    fn bundle_icon(img: &mut Image, x0: i32, y0: i32) {
        fill(img, x0 + 104, y0 + 102, 16, 16, (20, 20, 20));
        fill(img, x0 + 106, y0 + 104, 12, 12, (250, 250, 250));
        fill(img, x0 + 108, y0 + 106, 8, 8, (20, 20, 20));
    }

    #[test]
    fn video_tiles_are_told_apart_from_photos_and_bundles() {
        let mut img = blank(840, 600);
        thumb(&mut img, GRID_LEFT, 225, 5); // 그냥 사진
        thumb(&mut img, GRID_LEFT + PITCH, 225, 9);
        video_pill(&mut img, GRID_LEFT + PITCH, 225); // 동영상
        thumb(&mut img, GRID_LEFT + 2 * PITCH, 225, 13);
        bundle_icon(&mut img, GRID_LEFT + 2 * PITCH, 225); // 묶음
        let cells = find_cells(&img);
        assert_eq!(cells.len(), 3);
        let kinds: Vec<bool> = cells.iter().map(|c| is_video_tile(&img, c)).collect();
        assert_eq!(kinds, [false, true, false]);
    }

    #[test]
    fn bright_or_dark_photos_are_not_videos() {
        let mut img = blank(840, 600);
        thumb(&mut img, GRID_LEFT, 225, 5);
        fill(&mut img, GRID_LEFT, 225, CELL, CELL, (250, 250, 250)); // 흰 이미지
        fill(&mut img, GRID_LEFT + PITCH, 225, CELL, CELL, (15, 15, 15)); // 어두운 사진 (흰 점 없음)
        let cells = find_cells(&img);
        assert!(cells.iter().all(|c| !is_video_tile(&img, c)));
    }

    #[test]
    fn last_row_is_found_when_there_is_no_selection_bar() {
        // 선택 바가 없으면 마지막 줄이 창 아래까지 내려온다 (y 440..562 가 바 자리(538)에 걸쳐도 칸이다)
        let mut img = blank(840, 600);
        thumb(&mut img, GRID_LEFT, 440, 5);
        assert_eq!(find_cells(&img).len(), 1);
        // 바가 있으면 그 자리에 걸친 칸은 쓰지 않는다
        fill(&mut img, 800 - 6, 570 - 6, 12, 12, (30, 30, 30));
        fill(&mut img, 760 - 6, 570 - 6, 12, 12, (30, 30, 30));
        assert!(find_cells(&img).is_empty());
    }

    #[test]
    fn empty_drawer_has_no_cells() {
        assert!(find_cells(&blank(840, 600)).is_empty());
    }

    #[test]
    fn progress_popup_has_yellow_bar() {
        let mut img = blank(300, 200);
        assert!(!save_in_progress(&img), "결과 팝업에는 노란 막대가 없다");
        fill(&mut img, 24, 127, 250, 3, (255, 215, 0)); // 진행 막대 (y≈127)
        assert!(save_in_progress(&img));
    }

    #[test]
    fn selection_bar_needs_both_icons() {
        let mut img = blank(840, 600);
        assert!(!selection_bar_visible(&img));
        fill(&mut img, 800 - 6, 570 - 6, 12, 12, (30, 30, 30));
        assert!(!selection_bar_visible(&img), "다운로드 아이콘만 있으면 레이아웃이 다른 것으로 본다");
        fill(&mut img, 760 - 6, 570 - 6, 12, 12, (30, 30, 30));
        assert!(selection_bar_visible(&img));
        // 영상이 섞이면 전달 아이콘이 회색이 된다 (실측)
        let mut gray = blank(840, 600);
        fill(&mut gray, 800 - 6, 570 - 6, 12, 12, (30, 30, 30));
        fill(&mut gray, 760 - 6, 570 - 6, 12, 12, (190, 190, 190));
        assert!(selection_bar_visible(&gray));
        // 아무것도 없는 흰 자리는 아니다
        let mut blank_share = blank(840, 600);
        fill(&mut blank_share, 800 - 6, 570 - 6, 12, 12, (30, 30, 30));
        assert!(!selection_bar_visible(&blank_share));
        assert_eq!(download_icon(&img), (800, 570));
    }
}
