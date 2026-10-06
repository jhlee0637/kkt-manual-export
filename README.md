# 카카오톡 대화 수동 추출 프로그램
- 카카오톡 PC 대화를 지정된 스키마에 맞춰 저장한다.

## 고지
> **주의:** 이 프로젝트는 카카오 및 카카오톡과 관련이 없는 비공식 도구이다.

- 카카오톡 PC 클라이언트의 화면 조작(단축키, 마우스, 창 크기 변경)으로 **본인 계정의 대화 내보내기**만 자동화한다. 로컬 데이터베이스를 복호화하거나 네트워크 프로토콜을 사용하지 않는다.
- 카카오톡 이용약관과 운영정책을 지킬 책임은 사용자에게 있다. 클라이언트 업데이트로 동작이 깨질 수 있다.
- 수집한 대화에는 다른 사람의 대화와 개인정보가 들어 있다. 대화방 참여자의 동의가 필요한지 확인하고, 결과(`data/`)는 저장소나 외부에 올리지 않는다.
- 실행 중에는 마우스와 키보드를 점유한다. 마우스를 움직이면 일시정지하고(손을 떼면 2초 뒤 이어간다), `Ctrl+D` 로 중단할 수 있다.
- 소프트웨어는 어떠한 보증 없이 있는 그대로 제공된다. 라이선스는 [LICENSE](LICENSE) (MIT) 를 따른다.

## 파일구조
```bash
kkt-manual-exporter
├── AGENT.md                      # 개발 에이전트를 위한 규칙
├── README.md
├── LICENSE                       # MIT
├── .github/workflows/macos.yml   # Mac 용 빌드와 골든 검증 (수동 실행, 릴리즈에 올리기)
├── scripts
│   └── mac-report.sh             # Mac 시험·보고용 (읽기 전용: 환경, 카카오톡 메뉴, 내보내기 형식)
├── docs
│   ├── SCHEMA.md                 # 저장 형식과 판정 규칙의 기준 문서
│   └── RELEASE.md                # 릴리즈 계획 초안
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
│           ├── kkt-win           # 카카오톡 조작 (Windows 전용): 내보내기 수집, 서랍 사진 저장, 화면 분석
│           └── kkt-cli           # `kkt` 실행 파일 (골든 CLI 규격 + collect, photos, 더블클릭 안내 마당)
│
├── tests                         # 목적별 테스트
│   ├── python                    # Python 구현의 정확성 (내부 함수 단위)
│   └── golden                    # 구현 간 동등성 (Python, Rust 등 어느 구현이든)
│       ├── README.md             # 통과 조건, CLI 규격, 이식 시 주의점
│       └── scenarios             # 시나리오 26개 (모두 합성 데이터)
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
    - 사용자가 마우스를 움직이면 일시정지하고, 마우스가 2초 멈추면 이어간다 (봇이 둔 마우스 위치에서 5px 넘게 벗어나면 사용자 입력으로 본다)
- 암호화는 2차 구현 목표로.
- 더블클릭으로 쓸 수 있어야 한다: 인자 없이 `kkt.exe` 를 실행하면 안내 마당이 뜬다 (방 선택 → 수집 → 정리 → 사진 저장·연결).
    - 저장 위치는 `다운로드\kkt-manual-export-archive` (`archive/`, `exports/`)
    - 사진은 아직 연결되지 않은 사진 메시지 수만큼만 최신 사진을 받는다
    - 설정(결과 폴더, 다운로드 폴더, 동영상 보관)은 주석이 달린 설정 파일 `config.toml` 에 기억하고, 파일 위치는 화면 맨 위(상자 안)에 보여 준다. 경로는 항상 완전한 한 줄이라 복사할 수 있다
    - 입력은 한 번에 하나만(방 번호 / `o` 결과 폴더 열기 / `s` 설정 / `q` 종료). 용어는 수집·정리·다운로드·보관으로 통일했다
    - 사람이 정해야 할 때(기록이 크게 줄었을 때, 모호한 방, 이름 변경 의심, 동영상 보관)는 묻는다. 입력이 없으면 가장 안전한 쪽으로 한다
    - 여러 방을 한 번에 수집하고, 결과 폴더를 바로 열 수 있다
- Mac 은 카카오톡 조작을 아직 못 한다. 직접 내보낸 TXT 를 골라 정리하는 모드만 있다 (Mac 관측 후 수집을 만든다).

### 명령어
- Python 테스트: `python3 -m pytest` (저장소 루트)
- Rust: `cd src/rust && cargo test --release`, 빌드는 `cargo build --release`
- 골든: `python3 tests/golden/run.py --cmd "src/rust/target/release/kkt"`, 최신 확인은 `python3 tests/golden/build.py --check`
- CLI: `PYTHONPATH=src/python python3 -m kkt --conversation {방ID} ingest {TXT}`. 기본 아카이브는 `data/archive`
- Windows 쪽 수집 실행은 [dev-environment](references/dev-environment.md) 참고
- 더블클릭 사용: `kkt.exe` (인자 없음). 명령줄: `kkt collect --title {방 제목} --out {폴더}`, `kkt photos --title {방 제목} [--newest N] [--attach] [--no-videos] [--dry-run]`, `kkt attach {폴더} [--no-videos]`
- Mac 시험·보고: `bash scripts/mac-report.sh` (결과는 `~/Desktop/kkt-mac-report.txt`)
- 릴리즈: [릴리즈 페이지](https://github.com/jhlee0637/kkt-manual-export/releases). Mac 빌드는 GitHub Actions 의 `macos` 워크플로를 수동 실행한다
