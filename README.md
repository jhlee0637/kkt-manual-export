# 카카오톡 대화 수동 추출 프로그램
- 카카오톡 PC 대화를 지정된 스키마에 맞춰 저장한다.

## 파일구조
```bash
kkt-manual-exporter
├── AGENT.md                      # 개발 에이전트를 위한 규칙
├── README.md
├── docs
│   └── SCHEMA.md                 # 저장 형식과 판정 규칙의 기준 문서
├── references
│   ├── other-projects            # 비교용으로 클론한 외부 저장소 2개 (gitignore, 실행 금지)
│   └── other-projects-review.md  # 위 2개를 읽고 정리한 비교 문서
│
├── src                           # 언어별 소스. python 이 정답(참조 구현), rust 가 이식
│   ├── python
│   │   └── kkt                   # Python 패키지 `kkt` (import kkt, python -m kkt)
│   │       └── win               # Windows 수집 계층 (카카오톡 창 조작)
│   └── rust
│       ├── Cargo.toml, Cargo.lock
│       ├── README.md             # Rust 구현 안내 (빌드, 검증, 알려진 차이)
│       └── crates
│           ├── kkt-core          # 핵심 로직 (OS 무관). Python 구현의 이식
│           │   ├── src
│           │   └── tests
│           │       └── data                # 정답 526개와 그것을 만드는 생성기
│           └── kkt-cli           # `kkt` 실행 파일 (골든이 요구하는 CLI 규격)
│
├── tests                         # 목적별 테스트
│   ├── python                    # Python 구현의 정확성 (내부 함수 단위)
│   └── golden                    # 구현 간 동등성 (Python, Rust 등 어느 구현이든)
│       ├── README.md             # 통과 조건, CLI 규격, 이식 시 주의점
│       └── scenarios             # 시나리오 23개 (모두 합성 데이터)
│           └── room_rename       # 시나리오 하나의 예
│               ├── steps.json    # 실행할 CLI 호출 순서
│               ├── e1.txt, e2.txt, e3.txt   # 입력 내보내기
│               └── expected      # 기대 결과: results.json, tree.json, events/<방ID>.jsonl
│
└── data                          # 실제 대화 데이터. gitignore, 절대 올리지 않음
    ├── raw                       # 내보내기 TXT를 직접 모아 두는 곳 (입력)
    ├── attachments/image         # 저장한 사진 원본을 직접 모아 두는 곳 (입력)
    └── archive                   # 정리 결과 (CLI 기본 출력 위치)
        └── <방ID>                # 대화방 하나
            ├── events.jsonl      # 이벤트 로그 (유일한 원본)
            ├── raw               # 반영한 TXT 사본 (삭제하지 않음)
            └── attachments/image/<sha256 앞 2자>/   # attach 후 생김. 사진 보관본
```


### 결정사항
- Rust 이식
    - 가벼운 실행 파일 크기가 최우선
    - WIN/Mac 지원해야
    - GUI는 엔진과 분리(후보는 Tauri).
- 테스트는 저장소에 두고 배포(release)에서만 제외한다.
- 상대방이 자기 프로필 이름을 바꾼 경우는 범위 밖이다. 운영 중 발생하면 대응한다.
- 마우스가 봇에게 넘어가되, 최대한 빠르게 수행하도록 자동화
    - 키보드 명령어를 통한 긴급 탈출 구현 (ctrl+d)
- 암호화는 2차 구현 목표로.

### 명령어
- Python 테스트: `python3 -m pytest` (저장소 루트)
- Rust: `cd src/rust && cargo test --release`, 빌드는 `cargo build --release`
- 골든: `python3 tests/golden/run.py --cmd "src/rust/target/release/kkt"`, 최신 확인은 `python3 tests/golden/build.py --check`
- CLI: `PYTHONPATH=src/python python3 -m kkt --conversation {방ID} ingest {TXT}`. 기본 아카이브는 `data/archive`
- Windows 쪽 수집 실행은 [dev-environment](references/dev-environment.md) 참고
