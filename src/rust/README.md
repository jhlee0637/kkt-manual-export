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
텍스트 정리 뒤에 **아직 사진 파일과 연결되지 않은 사진 메시지 수만큼** 서랍의 최신 타일을 고르고(타일은 메시지 단위, `사진 16장` 묶음은 타일 하나) 저장해서 메시지에 연결한다
(이미 받은 사진을 다시 받지 않는다). 저장 중 팝업은 끝날 때까지 기다린다 (`Esc` 로 취소되기 때문).
실측: 타일 3개, 사진 20장을 8초에 저장하고 20장 모두 연결했다. 타일이 여러 줄이라 스크롤해야 할 때와 동영상은 미확인이다.

## 화면과 설정
시작 화면은 프로젝트 이름과 버전을 제목으로 한 상자다. 경로는 줄마다 **완전한 한 줄**이라 그대로 복사해서 쓸 수 있고(줄이지 않는다),
상자는 가장 긴 경로에 맞춰 넓어진다. 터미널이 상자보다 좁으면 선 없이 줄만 보여 준다. 한글은 두 칸으로 계산한다.

```
┌─ kkt-manual-export-v0.3.0 ─────────────────────────────────────┐
│  C:\Users\a\AppData\Roaming\kkt-manual-export\config.toml        │
│  ────────────────────────────────────────────────────────────  │
│  결과 폴더        C:\Users\a\Downloads\kkt-manual-export-archive   │
│                   ├─ archive\    # 정리한 대화                  │
│                   └─ exports\    # 내보낸 TXT                   │
│  다운로드 폴더    C:\Users\a\Documents\카카오톡 받은 파일          │
│  옵션                                                          │
│  동영상 보관      [물어보기]  항상 보관  보관 안 함            │
└────────────────────────────────────────────────────────────────┘
```

입력은 **한 번에 하나만**: 방 번호(`1, 3, 4` · `1-3` · `a` · `all`) / `o` 결과 폴더 열기 / `s` 설정 / `q` 종료.
`1s` 처럼 섞어 쓰면 거절하고, 빈 Enter 는 종료하지 않고 목록을 다시 불러온다.

용어 (한 단어는 한 가지 뜻만): **수집** = 카카오톡에서 내보내기(TXT)를 받는 것, **정리** = 받은 TXT 를 아카이브 기록에 반영하는 것,
**다운로드** = 서랍에서 사진·동영상 파일을 컴퓨터로 받는 것, **보관** = 다운로드한 파일을 결과 폴더에 복사하고 메시지에 연결하는 것,
**결과 폴더** = `archive\`(정리한 대화)와 `exports\`(내보낸 TXT)가 있는 폴더, **다운로드 폴더** = 카카오톡이 파일을 내려받는 폴더,
**저장** = 설정 파일에 값을 쓰는 것에만 쓴다.

- **설정 파일** `config.toml`: 주석에 선택지를 적어 두어서 파일만 열어 봐도 고를 수 있는 값을 안다. 경로는 작은따옴표 안에 그대로(`\` 를 두 번 쓰지 않는다).
  직접 고쳐도 되고, 프로그램이 저장할 때는 주석까지 포함해 파일 전체를 다시 쓴다. 위치는 화면 맨 위에 보인다
  (Windows `%APPDATA%\kkt-manual-export\`, macOS `~/Library/Application Support/kkt-manual-export/`, 그 밖 `~/.config/kkt-manual-export/`. `KKT_CONFIG` 로 지정 가능).
  항목: `result_dir`, `download_dir`, `videos`(`ask`/`keep`/`skip`). 옛 `config.json`(0.3.0, `output_dir`·`kakao_photo_dir`)은 읽어서 옮기고 지우지 않는다.
  잘못된 값·줄은 이유를 화면에 알리고 기본값으로 동작한다.
- **안내 파일**: 결과 폴더에 `README.md`(파일 설명, 채팅방 목록, 백업 안내)와 `AGENTS.md`(AI 에이전트가 이 폴더를 읽을 때의 규칙)를 만든다.
  원본은 `src/rust/crates/kkt-cli/templates/` 에 있고 실행 파일에 들어 있다. 정리를 마칠 때마다 없으면 만들고, `README.md` 의
  `<!-- rooms:begin -->` ~ `<!-- rooms:end -->` 사이(채팅방 이름·폴더·메시지 수·마지막 정리)만 새로 쓴다. 그 밖에 직접 쓴 글과 `AGENTS.md` 는 건드리지 않는다.
  표시를 지우면 목록을 쓰지 않는다. 전체를 새 안내문으로 바꾸는 것은 설정의 `g`(안내 파일 다시 만들기, 확인 질문 있음)뿐이다.
- **사용자에게 묻기**: 반영 전에 임시 복사본에서 미리 돌려 보고 필요하면 묻는다. 입력이 없으면 가장 안전한 쪽(정리하지 않기, 보류, 동영상 보관 안 함)이다.
  - 이 PC 의 기록이 크게 줄었을 때(`mass_loss`, 예: QR 1회용 로그인): 정리하지 않기(권장) / 그래도 정리하기
  - 방이 모호할 때: 기존 방에 이어 붙이기 / 새 방으로 따로 저장 / 정리하지 않기
  - 이름 변경이 의심되지만 근거가 약할 때: 같은 사람인지
  - 보관하지 않은 동영상이 있을 때(설정이 "물어보기"): 이번에만/항상 보관, 이번에만/항상 보관하지 않기
- **여러 방 수집**: Ctrl+D 로 중단하면 남은 방은 건너뛴다.
- 동영상: 보관하면 `attachments/video/` 에 복사하고 `동영상` 메시지와 연결한다. 보관하지 않으면 서랍에서 동영상 타일을 고르지도 않는다
  (영상은 화면의 어두운 알약과 흰 재생 시간으로 알아본다). `kkt photos --dry-run` 은 아무것도 누르지 않고 서랍을 훑어 타일 수와 동영상 수를 센다.

## 마우스를 움직이면 일시정지
봇이 마우스를 옮길 때마다 기대 위치를 기록하고, 모든 대기와 입력 직전에 커서가 그 자리(오차 5px 이내)에 있는지 확인한다.
벗어나면 사용자가 움직인 것으로 보고 일시정지하며, 마우스가 2초 멈추면 이어간다 (`Ctrl+D` 는 일시정지 중에도 먹는다).
- 사진 저장(마우스를 쓰는 작업): 일시정지 뒤 **현재 단계를 처음부터 다시** 한다. 사용자가 클릭했을 수 있어서 화면 상태를 장담할 수 없고,
  각 단계가 시작할 때 화면에서 상태를 다시 읽기 때문이다 (서랍이 열려 있으면 그대로 쓰고, 선택은 화면의 체크로 다시 읽는다).
- 내보내기 수집(키보드만 쓰는 작업): 일시정지 뒤 그대로 이어간다.
- 결과 JSON 의 `pauses` 에 일시정지한 횟수가 남는다. 실측: 사진 저장 중과 내보내기 중에 다른 프로세스가 커서를 옮기게 해서 둘 다 재개 후 정상 완료.

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
