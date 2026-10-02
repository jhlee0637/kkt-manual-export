//! Python difflib 가 만든 정답(tests/data/difflib_cases.json)과 이식한 SequenceMatcher 를 대조한다.

use kkt_core::difflib::{SequenceMatcher, Tag};
use serde_json::Value;

fn tag_name(t: Tag) -> &'static str {
    match t {
        Tag::Replace => "replace",
        Tag::Delete => "delete",
        Tag::Insert => "insert",
        Tag::Equal => "equal",
    }
}

#[test]
fn matches_python_difflib_on_all_cases() {
    let text = include_str!("data/difflib_cases.json");
    let cases: Vec<Value> = serde_json::from_str(text).unwrap();
    assert!(cases.len() > 500);
    for (n, c) in cases.iter().enumerate() {
        let a: Vec<String> = c["a"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
        let b: Vec<String> = c["b"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
        let sm = SequenceMatcher::new(&a, &b);
        let blocks: Vec<Vec<u64>> = sm.get_matching_blocks().iter().map(|&(i, j, k)| vec![i as u64, j as u64, k as u64]).collect();
        let want_blocks: Vec<Vec<u64>> =
            c["blocks"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).collect()).collect();
        assert_eq!(blocks, want_blocks, "case {n}: matching_blocks a={a:?} b={b:?}");
        let ops: Vec<(String, u64, u64, u64, u64)> = sm
            .get_opcodes()
            .iter()
            .map(|&(t, i1, i2, j1, j2)| (tag_name(t).to_string(), i1 as u64, i2 as u64, j1 as u64, j2 as u64))
            .collect();
        let want_ops: Vec<(String, u64, u64, u64, u64)> = c["opcodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let r = r.as_array().unwrap();
                (r[0].as_str().unwrap().to_string(), r[1].as_u64().unwrap(), r[2].as_u64().unwrap(), r[3].as_u64().unwrap(), r[4].as_u64().unwrap())
            })
            .collect();
        assert_eq!(ops, want_ops, "case {n}: opcodes a={a:?} b={b:?}");
    }
}
