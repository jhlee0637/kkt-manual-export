# 개요
- 이 파일은 AI로 직접 수정을 금지한다.
- 이 파일은 개발에 참여하는 새 에이전트가 알아야할 내용, 지켜야할 규칙을 명시하고 있다.
- [README.md](README.md) 파일을 읽고 프로젝트를 파악할 것.

## Taxonomy
| 용어 | 뜻 |
|---|---|
| golden=골든 | 기대되는 프로그램 출력과 비교하기 위한 시나리오. |
| 종결 이벤트 | 트랜잭션을 닫는 이벤트: `export.ingested`, `attach.committed`, `state.committed` |
| 삭제 표식 | TXT의 `메시지가 삭제되었습니다.` 줄 |
| 소급 | 이름 변경 후 내보내기가 과거 메시지에도 새 이름을 찍는 것 |
| 서랍 | 카카오톡의 `채팅방 서랍`. 사진/동영상·파일·링크를 모아 보여 주는 창 |

## Skills


### 읽는 순서


### 코드는 어디에 있는가
- 저장소는 로컬 전용이며 원격이 없다. 위치는 `work-path`를 따르거나 사용자에게 확인한다.
- `src/python/kkt/`: Python 참조 구현(정답). `src/rust/`: Rust 이식
- `tests/golden/`: 두 구현이 함께 쓰는 계약. `tests/python/`: Python 단위 테스트
- `data/`: 대화 데이터. gitignore이며 절대 올리지 않는다
- `docs/`: 명세. `references/`: 참고 자료(외부 클론은 `references/other-projects/`, gitignore)
### 핵심 개념
- 내보내기(`Ctrl+S`) TXT를 직전 내보내기와 순서로 정렬해 이벤트 로그 `events.jsonl`을 만든다. 로그가 유일한 원본이고 상태는 로그에서 복원한다.
- 카카오톡 TXT에는 메시지 ID가 없다. 순서로 맞추고, 참가자는 이름이 아니라 `participant_id`로 비교한다.
- 모두에게 삭제는 삭제 표식이 남아 확정, 나에게서만 삭제는 흔적 없이 사라져 주체 불명, 수정은 후보로만 기록한다.
- 로컬에서 방 이름·멤버 이름을 바꾸면 내보내기가 과거 메시지에도 소급해 새 이름을 찍는다. 겹침 비율과 근거 개수로 판정하고, 애매하면 중단한다.
- 원칙: 관측한 것과 추정한 것을 구분하고, 모르면 추측하지 않고 `text`로 둔다.
- 상세는 저장소의 `docs/SCHEMA.md`.
### 두 구현과 골든
- Python이 정답(오라클), Rust가 이식이다.
- 골든은 구현이 같은지만 보장한다. 정확성은 단위 테스트와 실측으로 지킨다.
- 동작을 바꾸는 순서: 골든 재생성 → `git diff` 리뷰 → Python과 Rust 모두 통과.
- Rust 이식 주의: `difflib` 동작, 임계값은 `f64` 비교, JSON 바이트(구분자와 키 정렬).
- 테스트가 실제로 잡는지 값을 일부러 바꿔 확인한다(변이 시험).
### 명령어
- Python 테스트: `python3 -m pytest` (저장소 루트)
- Rust: `cd src/rust && cargo test --release`, 빌드는 `cargo build --release`
- 골든: `python3 tests/golden/run.py --cmd "src/rust/target/release/kkt"`, 최신 확인은 `python3 tests/golden/build.py --check`
- CLI: `PYTHONPATH=src/python python3 -m kkt --conversation {방ID} ingest {TXT}`. 기본 아카이브는 `data/archive`
- Windows 쪽 수집 실행은 [dev-environment](references/dev-environment.md) 참고

### 카카오톡 클라이언트에서 알아낸 것 (요약)
- 메시지 목록은 직접 그리는 컨트롤이라 UI Automation으로 읽을 수 없다. `SendInput`은 되고 `PostMessage`로 키·클릭 전달은 안 된다. `PrintWindow` 캡처는 된다.
- 내보내기는 채팅창에서 `Ctrl+S`. 메뉴의 `대화 내용 모두 삭제`가 바로 인접하므로 방향키로 메뉴를 고르지 않는다.
- 완료 팝업은 별도 창이다. 포커스가 팝업일 때만 `Enter`를 보낸다.
- 사진은 서랍에서 썸네일의 선택 원을 눌러 저장한다. 파일명에 초·밀리초가 들어간다.
- Windows 클라이언트에서만 확인했고 Mac은 미확인이다. 상세는 [kakaotalk-ui-observations](references/kakaotalk-ui-observations.md).

### 환경과 함정 (요약)
- WSL에서 개발하고 카카오톡 조작은 Windows Python으로 한다.
- WSL에서 `sudo`를 쓸 수 없고 구글 드라이브 `/mnt/g`는 끊길 수 있다.
- 외부에서 받은 코드의 실행은 차단된다. 사용자 승인이 필요하다. 상세는 [dev-environment](references/dev-environment.md).

### 지켜야 할 규칙
- 대화 원문·사진·실명을 깃허브에 올리지 않는다.
- 테스트는 합성 데이터만 쓴다.
- 키를 보내기 전에 포커스를 확인하고, 입력을 점유하기 전에 사용자에게 알린다.
- 실패와 미검증을 숨기지 않고 수치로 보고한다.
