# 골든 시나리오 (이식용 계약)

Python 구현을 정답(오라클)으로 삼아, **같은 입력에 같은 출력**을 내는지 검증하는 CLI 수준의 계약이다.
다른 언어(Rust 등)로 이식한 구현은 이 시나리오를 그대로 통과해야 한다.

## 구성

```
tests/golden/
  build.py            시나리오 생성기. 입력(TXT, 사진, steps.json)을 코드로 만들고 expected/ 를 채운다
  runner.py           steps.json 실행기 (Python 참조 구현)
  scenarios/<이름>/
    steps.json        실행할 단계 목록 + 설명
    e1.txt ...        입력 내보내기 (UTF-8 CRLF, BOM/CP949 변형 포함)
    photos/           첨부 시나리오의 사진 파일
    expected/
      results.json    단계별 종료 코드, 표준 출력(JSON), 오류 코드
      tree.json       최종 아카이브의 파일 목록과 sha256
      events/<대화방>.jsonl   대화방별 이벤트 로그 원문 (바이트 단위로 같아야 함)
```

입력은 모두 합성 데이터다. 실제 대화나 사진은 들어 있지 않다.

## steps.json

- `{"argv": [...]}` : CLI 호출. `{dir}` 은 시나리오 폴더, `{archive}` 는 작업용 아카이브 루트로 치환된다.
- `{"op": "write_file"|"append_text", "path": ..., "text": ...}` : 크래시 꼬리, 옛 로그 심기.

## 통과 조건 (세 가지 모두)

1. **results.json**: 단계마다 `exit`(종료 코드), `stdout`(JSON으로 파싱한 값), `error_codes` 가 같다.
   - 표준 출력은 문자열이 아니라 **파싱한 JSON 값**으로 비교한다 (키 순서 무관).
   - 표준 오류는 `[중단:<code>] 메시지` 줄에서 **code 만** 비교한다. 메시지 문장은 구현마다 달라도 된다.
2. **tree.json**: 아카이브의 모든 파일 경로와 sha256 이 같다.
3. **events/<대화방>.jsonl**: **바이트 단위로 같다.** 이벤트 JSON 은 키를 정렬하고(`sort_keys`) 한글을 이스케이프하지
   않으며(`ensure_ascii=False`) 줄 끝은 `\n`, 각 줄 뒤에 개행을 붙인다. ID(`kmsg_…` 등)는 SHA-1 로 정해지므로
   해시 입력 문자열까지 같아야 한다 (`kkt/reconcile.py` 의 `new_id`, `kkt/link.py` 의 `new_conversation_id`).

## 오류 코드 (안정된 계약)

`empty_export`, `mass_loss`, `bad_encoding`, `participant_not_found`, `participant_same`,
`explicit_conversation_mismatch`, `room_title_ambiguous`, `room_rename_ambiguous`, `schema_too_old`, `usage`

## 이식할 때 특히 조심할 곳

- **내보내기 간 정렬**은 Python `difflib.SequenceMatcher(autojunk=False)` 의 동작(가장 긴 일치 블록부터 재귀적으로 쪼갬,
  동률이면 앞쪽 우선)에 의존한다. Myers 계열 diff 크레이트는 같은 결과를 보장하지 않는다.
  `combined_realworld`, `member_rename`, `edit_candidate` 가 이 차이를 잡는다.
- 인코딩 판별은 `utf-8-sig` → `cp949` 순서다. (`encoding_and_format`)
- 시각은 항상 한국 시간(+09:00) 문자열로 만든다. 시스템 시계는 쓰지 않는다.

## 사용

```bash
python3 -m pytest tests/test_golden.py          # 현재 구현이 계약을 지키는지
python3 tests/golden/build.py --check           # 커밋된 골든이 최신인지
python3 tests/golden/build.py                   # 동작을 일부러 바꿨을 때 기대 결과를 다시 만든다 (git diff 로 리뷰)
```
