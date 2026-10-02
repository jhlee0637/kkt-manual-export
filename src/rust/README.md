# kkt (Rust)

Python 구현(`src/python/kkt/`)의 이식. 기준은 `tests/golden/` 의 골든 시나리오다: 같은 입력에 같은 출력(종료 코드, 표준 출력 JSON,
오류 코드, 아카이브 파일 해시, `events.jsonl` **바이트**)을 내야 한다.

```
src/rust/
  crates/kkt-core/   핵심 로직 (OS 무관): parse, state, reconcile, link, archive, attach, difflib, pyfmt
  crates/kkt-win/    카카오톡 창 조작 수집 계층 (Windows 전용. 다른 OS 에서는 "Windows 에서만 동작한다"로 실패). Win32 는 직접 선언
  crates/kkt-cli/    `kkt` 바이너리 (골든이 요구하는 CLI 규격 + `collect`)
```

## 더블클릭으로 쓰기
인자 없이 `kkt.exe` 를 실행(더블클릭)하면 안내 마당이 뜬다 (`kkt wizard` 도 같다). 열려 있는 카카오톡 방 목록에서 번호를 고르면
수집(`collect`)부터 정리(`ingest`)까지 한 번에 하고, 결과를 문장으로 보여 준다. Windows 전용이다.
- 저장 위치: `다운로드\kkt-manual-export-archive` (`archive/` 정리 결과, `exports/` 내보내기 TXT)
- 같은 방을 다시 고르면 달라진 것만 반영한다.
- 인자를 주면 지금처럼 명령줄로 동작한다.

`kkt photos --title <방 제목> [--newest N] [--save-dir <폴더>] [--attach]` 는 서랍의 사진을 저장한다 (Windows 전용). 더블클릭 안내 마당은
텍스트 정리 뒤에 **아직 사진 파일과 연결되지 않은 사진 메시지 수만큼** 최신 사진을 저장하고 메시지에 연결한다 (이미 받은 사진을 다시 받지 않는다).
실측은 사진 2장까지다. 여러 줄 스크롤과 동영상은 미확인이다.

## 빌드와 검증

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src/rust && cargo build --release                   # target/release/kkt  (opt-level z, lto, strip)
cargo test                                             # 단위 테스트 + difflib 차분 테스트
cd ../.. && python3 tests/golden/run.py --cmd "src/rust/target/release/kkt"   # 골든 전체 (어긋난 지점을 보여 줌)
KKT_GOLDEN_CMD="src/rust/target/release/kkt" python3 -m pytest tests/python/test_golden.py
```

## 두 겹의 안전장치

1. **골든 시나리오** (CLI 수준): 동작이 Python 과 같은지. 임계값 경계와 동률 처리까지 시나리오로 고정되어 있다.
2. **difflib 차분 테스트** (`crates/kkt-core/tests/difflib_oracle.rs`): `SequenceMatcher` 이식을 Python 이 만든 정답
   (526개 사례, `tests/data/difflib_cases.json`)과 대조한다. 정답 데이터는 `tests/data/gen_difflib_cases.py` 로 다시 만든다.

## Python 과 같게 맞춘 곳 (바이트가 달라지는 곳)

- 이벤트 JSON: 키 정렬, 구분자 `", "` / `": "`, 한글 비이스케이프 (`pyfmt::dumps`)
- 내보내기 간 정렬: `difflib.SequenceMatcher(autojunk=False)` 의 가장 긴 일치 블록 우선, 동률은 앞쪽 우선 (`difflib.rs`)
- 임계값 비교: `f64` 로 `m/c >= 0.6` (정수 비교로 바꾸면 경계에서 달라질 수 있다)
- 삽입 순서에 의존하는 순회: `IndexMap` (Python dict/Counter 와 같은 순서)
- 이벤트 방출 순서: `new_id` → `pid_for`(participant.observed) → `message.observed`
- 줄 구분은 `\r\n`, `\n`, `\r` 만이다 (U+2028 등은 본문)
- 경고 문구의 `{x!r}`: `pyfmt::repr`

## 알려진 차이와 한계

- `pyfmt::repr` 의 "출력 가능 문자" 판정은 근사다 (유니코드 범주표가 없어 흔한 비출력 문자만 이스케이프). 경고 문구에서만 쓰인다.
- 정규식의 `\d` 는 ASCII 숫자만 받는다 (Python 은 다른 문자 체계의 숫자도 받는다). 카카오톡 내보내기에서는 ASCII 만 나온다.
- cp949 디코딩은 `encoding_rs`(WHATWG euc-kr = 통합 완성형)다. Python `cp949` 와 극히 드문 바이트열에서 다를 수 있다.
- CLI 인자 처리는 argparse 의 일부만 흉내 낸다 (약어 옵션 `--arch` 등과 위치 인자 뒤섞기는 지원하지 않는다).
- 오류 메시지 문장은 구현마다 달라도 된다 (비교하는 것은 오류 코드뿐이다).
- 수집 계층(`kkt-win`)은 Python(`src/python/kkt/win/`)의 이식이다. Windows 에서 실제 카카오톡으로 확인했다:
  내보내기 성공(경고 없음, 창 크기 복원), `Ctrl+D` 중단(종료 코드 130, 대화상자 정리, 창 위치 그대로). 생성한 TXT 를 Python 과 Rust 가
  반영한 `events.jsonl` 은 바이트가 같았다. 미확인: 최소화·최대화 상태의 창, 배율 100%가 아닌 화면, 다른 PC.
- WSL 에서는 Windows 용 실행 파일을 링크할 수 없다. 표준 라이브러리가 `kernel32.lib` 등 Windows SDK 라이브러리를 요구한다.
  타입 확인만 `cargo check --release --target x86_64-pc-windows-msvc` 로 한다. 빌드는 Windows 에서 한다
  (`rustup` + Visual Studio Build Tools 의 "C++를 사용한 데스크톱 개발"). 결과 `kkt.exe` 는 약 680KB.
- GUI 는 아직 없다.
