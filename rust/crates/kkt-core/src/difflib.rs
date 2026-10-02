//! Python `difflib.SequenceMatcher(None, a, b, autojunk=False)` 의 이식.
//!
//! 내보내기 간 정렬이 이 알고리즘의 정확한 동작에 의존한다. 가장 긴 일치 블록을 먼저 찾고 양쪽을 재귀적으로 쪼개며,
//! 길이가 같은 일치가 여럿이면 a 에서 가장 앞, 그다음 b 에서 가장 앞의 것을 고른다.
//! Myers 계열 diff 는 같은 결과를 보장하지 않으므로 대체하면 안 된다.
//! (junk 와 자동 junk 는 쓰지 않으므로 이식하지 않았다.)

use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Replace,
    Delete,
    Insert,
    Equal,
}

pub struct SequenceMatcher<'a, T> {
    a: &'a [T],
    b: &'a [T],
    b2j: HashMap<&'a T, Vec<usize>>,
}

impl<'a, T: Eq + Hash> SequenceMatcher<'a, T> {
    pub fn new(a: &'a [T], b: &'a [T]) -> Self {
        let mut b2j: HashMap<&T, Vec<usize>> = HashMap::new();
        for (j, x) in b.iter().enumerate() {
            b2j.entry(x).or_default().push(j);
        }
        SequenceMatcher { a, b, b2j }
    }

    /// a[alo..ahi] 와 b[blo..bhi] 사이의 가장 긴 일치 (i, j, size).
    fn find_longest_match(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for i in alo..ahi {
            let mut newj2len: HashMap<usize, usize> = HashMap::new();
            if let Some(js) = self.b2j.get(&self.a[i]) {
                for &j in js {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let prev = if j == 0 { 0 } else { j2len.get(&(j - 1)).copied().unwrap_or(0) };
                    let k = prev + 1;
                    newj2len.insert(j, k);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            j2len = newj2len;
        }
        (besti, bestj, bestsize)
    }

    /// (i, j, size) 목록. 마지막은 항상 (len(a), len(b), 0) 이다. 인접한 블록은 합쳐진다.
    pub fn get_matching_blocks(&self) -> Vec<(usize, usize, usize)> {
        let (la, lb) = (self.a.len(), self.b.len());
        let mut queue = vec![(0usize, la, 0usize, lb)];
        let mut blocks: Vec<(usize, usize, usize)> = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.find_longest_match(alo, ahi, blo, bhi);
            if k > 0 {
                blocks.push((i, j, k));
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        blocks.sort();
        let (mut i1, mut j1, mut k1) = (0usize, 0usize, 0usize);
        let mut out = Vec::new();
        for (i2, j2, k2) in blocks {
            if i1 + k1 == i2 && j1 + k1 == j2 {
                k1 += k2;
            } else {
                if k1 > 0 {
                    out.push((i1, j1, k1));
                }
                i1 = i2;
                j1 = j2;
                k1 = k2;
            }
        }
        if k1 > 0 {
            out.push((i1, j1, k1));
        }
        out.push((la, lb, 0));
        out
    }

    /// (tag, i1, i2, j1, j2) 목록.
    pub fn get_opcodes(&self) -> Vec<(Tag, usize, usize, usize, usize)> {
        let (mut i, mut j) = (0usize, 0usize);
        let mut answer = Vec::new();
        for (ai, bj, size) in self.get_matching_blocks() {
            let tag = if i < ai && j < bj {
                Some(Tag::Replace)
            } else if i < ai {
                Some(Tag::Delete)
            } else if j < bj {
                Some(Tag::Insert)
            } else {
                None
            };
            if let Some(t) = tag {
                answer.push((t, i, ai, j, bj));
            }
            i = ai + size;
            j = bj + size;
            if size > 0 {
                answer.push((Tag::Equal, ai, i, bj, j));
            }
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(a: &[&str], b: &[&str]) -> Vec<(Tag, usize, usize, usize, usize)> {
        SequenceMatcher::new(a, b).get_opcodes()
    }

    #[test]
    fn identical_and_empty() {
        assert_eq!(ops(&["a", "b"], &["a", "b"]), vec![(Tag::Equal, 0, 2, 0, 2)]);
        assert!(ops(&[], &[]).is_empty());
        assert_eq!(ops(&[], &["x"]), vec![(Tag::Insert, 0, 0, 0, 1)]);
        assert_eq!(ops(&["x"], &[]), vec![(Tag::Delete, 0, 1, 0, 0)]);
    }

    /// 실측 사례: 삭제 표식 d 가 두 개 연속일 때 가장 긴 블록 (d, p8) 이 뒤쪽 d 와 짝지어진다.
    #[test]
    fn longest_block_wins_not_first_element() {
        let a = ["t1", "t2", "p7", "d", "p8"];
        let b = ["t1", "t22", "d", "d", "p8"];
        let blocks = SequenceMatcher::new(&a, &b).get_matching_blocks();
        assert_eq!(blocks, vec![(0, 0, 1), (3, 3, 2), (5, 5, 0)]);
    }
}
