//! 서랍에서 무엇을 받을지 계획한다: 몇 칸을 고르고, 파일이 몇 개 생길지, 동영상을 보관할지.
//!
//! 서랍은 사진과 동영상을 메시지 하나당 타일 하나로 최신순으로 보여 준다 (`사진 3장` 묶음도 타일 하나).
//! 아직 파일과 연결되지 않은 가장 오래된 메시지부터 최신까지가 대상이다.
//! 동영상은 크고 항상 원하는 것이 아니므로, 설정이 "물어보기"면 동영상이 대상에 있을 때 묻는다.

use crate::ingest_flow::Asker;
use crate::settings::Videos;

/// 서랍에 타일로 보이는 메시지 하나 (사진 또는 동영상).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRec {
    pub date: String,
    pub hhmm: String,
    /// 같은 분 안의 순서 (기록에 들어온 순서)
    pub order: usize,
    pub is_video: bool,
    /// 사진 메시지의 사진 수 (`사진 3장` 이면 3). 동영상은 1.
    pub count: usize,
    /// 아직 파일과 연결되지 않았다
    pub unlinked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// 서랍에서 고를 최신 칸 수 (동영상을 보관하지 않으면 사진 칸만 센다)
    pub tiles: usize,
    /// 저장될 것으로 기대하는 파일 수
    pub files: usize,
    pub keep_videos: bool,
}

fn fetch(media: &[MediaRec], keep_videos: bool) -> (usize, usize, usize) {
    // 가장 오래된 "아직 연결 안 된" 대상: 보관하면 사진·동영상 모두, 안 하면 사진만
    let Some(oldest) = media.iter().position(|m| m.unlinked && (keep_videos || !m.is_video)) else { return (0, 0, 0) };
    let range = &media[oldest..];
    let videos = range.iter().filter(|m| m.is_video).count();
    let tiles = if keep_videos { range.len() } else { range.len() - videos };
    let files = range.iter().filter(|m| keep_videos || !m.is_video).map(|m| m.count).sum();
    (tiles, files, videos)
}

/// 계획을 세운다. 반환: `(계획 또는 받을 것이 없음, 설정에 새로 저장할 값)`.
pub fn plan(media: &[MediaRec], mode: Videos, ask: &mut dyn Asker) -> (Option<Plan>, Option<Videos>) {
    let mut sorted = media.to_vec();
    sorted.sort_by(|a, b| (&a.date, &a.hhmm, a.order).cmp(&(&b.date, &b.hhmm, b.order)));
    let (kt, kf, kv) = fetch(&sorted, true);
    let (st, sf, _) = fetch(&sorted, false);
    let skip_plan = (st > 0).then_some(Plan { tiles: st, files: sf, keep_videos: false });
    if kv == 0 {
        return (skip_plan, None); // 받을 동영상이 없으면 묻지 않는다
    }
    let (keep, remember) = match mode {
        Videos::Keep => (true, None),
        Videos::Skip => (false, None),
        Videos::Ask => {
            let q = format!(
                "서랍에 아직 보관하지 않은 동영상이 {kv}개 있습니다. 아카이브에 보관할까요?\n  동영상은 크기가 커서 저장 폴더와 아카이브에 파일이 그대로 복사됩니다. 보관하지 않아도 메시지는 그대로 기록됩니다."
            );
            let opts = vec![
                "이번에는 보관하지 않기".to_string(),
                "이번에는 보관하기".to_string(),
                "앞으로 항상 보관하기 (설정에 저장)".to_string(),
                "앞으로 보관하지 않기 (설정에 저장)".to_string(),
            ];
            match ask.choose(&q, &opts, 0) {
                1 => (true, None),
                2 => (true, Some(Videos::Keep)),
                3 => (false, Some(Videos::Skip)),
                _ => (false, None),
            }
        }
    };
    let plan = if keep { (kt > 0).then_some(Plan { tiles: kt, files: kf, keep_videos: true }) } else { skip_plan };
    (plan, remember)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Script(VecDeque<usize>, usize);

    impl Asker for Script {
        fn choose(&mut self, _q: &str, _o: &[String], default: usize) -> usize {
            self.1 += 1;
            self.0.pop_front().unwrap_or(default)
        }
    }

    fn rec(hhmm: &str, order: usize, is_video: bool, count: usize, unlinked: bool) -> MediaRec {
        MediaRec { date: "2026-10-06".into(), hhmm: hhmm.into(), order, is_video, count, unlinked }
    }

    fn photo(h: &str, o: usize, n: usize, unlinked: bool) -> MediaRec {
        rec(h, o, false, n, unlinked)
    }

    fn video(h: &str, o: usize, unlinked: bool) -> MediaRec {
        rec(h, o, true, 1, unlinked)
    }

    #[test]
    fn nothing_to_fetch_when_everything_is_linked() {
        let m = [photo("08:07", 0, 16, false), video("08:50", 1, false)];
        let mut ask = Script(VecDeque::new(), 0);
        assert_eq!(plan(&m, Videos::Ask, &mut ask), (None, None));
        assert_eq!(ask.1, 0);
    }

    #[test]
    fn photos_only_are_planned_without_asking() {
        let m = [photo("08:07", 0, 16, false), photo("08:07", 1, 2, true), photo("08:08", 2, 2, true)];
        let mut ask = Script(VecDeque::new(), 0);
        let (p, remember) = plan(&m, Videos::Ask, &mut ask);
        assert_eq!(p, Some(Plan { tiles: 2, files: 4, keep_videos: false }), "동영상이 없으면 보관 여부는 상관없다");
        assert_eq!((remember, ask.1), (None, 0));
    }

    #[test]
    fn keep_counts_videos_as_tiles_and_files() {
        // 사진 묶음(연결됨), 사진 2칸, 동영상, 사진 (모두 연결 안 됨은 아님)
        let m = [photo("08:07", 0, 16, false), photo("08:49", 1, 1, true), video("08:50", 2, true), photo("08:50", 3, 1, true)];
        let (p, _) = plan(&m, Videos::Keep, &mut Script(VecDeque::new(), 0));
        assert_eq!(p, Some(Plan { tiles: 3, files: 3, keep_videos: true }));
    }

    #[test]
    fn skip_leaves_video_tiles_out_of_the_count() {
        let m = [photo("08:49", 0, 1, true), video("08:50", 1, true), photo("08:50", 2, 1, true)];
        let (p, _) = plan(&m, Videos::Skip, &mut Script(VecDeque::new(), 0));
        assert_eq!(p, Some(Plan { tiles: 2, files: 2, keep_videos: false }), "동영상 칸은 고르지 않으니 사진 칸만 센다");
    }

    #[test]
    fn only_unlinked_videos_with_skip_means_nothing_to_download() {
        let m = [photo("08:49", 0, 1, false), video("08:50", 1, true)];
        assert_eq!(plan(&m, Videos::Skip, &mut Script(VecDeque::new(), 0)).0, None);
        let (p, _) = plan(&m, Videos::Keep, &mut Script(VecDeque::new(), 0));
        assert_eq!(p, Some(Plan { tiles: 1, files: 1, keep_videos: true }));
    }

    #[test]
    fn ask_mode_asks_once_and_default_is_not_to_keep() {
        let m = [photo("08:49", 0, 1, true), video("08:50", 1, true)];
        let mut ask = Script(VecDeque::new(), 0);
        let (p, remember) = plan(&m, Videos::Ask, &mut ask);
        assert_eq!(ask.1, 1);
        assert_eq!(p, Some(Plan { tiles: 1, files: 1, keep_videos: false }));
        assert_eq!(remember, None);
    }

    #[test]
    fn ask_answers_map_to_plan_and_remembered_setting() {
        let m = [photo("08:49", 0, 1, true), video("08:50", 1, true)];
        let run = |ans: usize| plan(&m, Videos::Ask, &mut Script(VecDeque::from([ans]), 0));
        assert_eq!(run(0), (Some(Plan { tiles: 1, files: 1, keep_videos: false }), None));
        assert_eq!(run(1), (Some(Plan { tiles: 2, files: 2, keep_videos: true }), None));
        assert_eq!(run(2), (Some(Plan { tiles: 2, files: 2, keep_videos: true }), Some(Videos::Keep)));
        assert_eq!(run(3), (Some(Plan { tiles: 1, files: 1, keep_videos: false }), Some(Videos::Skip)));
    }

    #[test]
    fn input_order_does_not_matter_and_time_order_does() {
        // 입력이 뒤섞여 있어도 (날짜, 분, 들어온 순서) 로 정렬한 뒤 가장 오래된 미연결부터 센다
        let m = [photo("09:00", 2, 1, true), photo("08:00", 0, 3, false), photo("08:30", 1, 1, true)];
        let (p, _) = plan(&m, Videos::Skip, &mut Script(VecDeque::new(), 0));
        assert_eq!(p, Some(Plan { tiles: 2, files: 2, keep_videos: false }));
    }
}
