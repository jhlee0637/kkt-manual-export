> **주의:** 이 PC(WSL + Windows) 기준이다. 다른 환경에서는 확인이 필요하다.
# 두 환경
- 개발: WSL(Ubuntu 24.04). Python 3.13, pytest, Rust(`rustup`, minimal 프로필)
- 카카오톡 조작: Windows Python 3.12. 표준 라이브러리(`ctypes`)만 쓰고 설치할 패키지가 없다.
# Windows 쪽 실행
- WSL에서 `powershell.exe`를 호출한다. 저장소 경로는 `\\wsl.localhost\{배포판}\{WSL 경로}` 형식의 UNC 경로다.
- 환경 변수 `PYTHONPATH`에 `{저장소}\src\python`, `PYTHONIOENCODING`에 `utf-8`을 준다.
- 수집 예: `python -m kkt.win.collect --title {방 제목} --out {TXT를 저장할 폴더}`. `--hold N`은 `Ctrl+D` 중단 시험용 대기다.
- 실행 중에는 마우스와 키보드를 점유한다. 시작 전에 사용자에게 알린다.
# 함정
- PowerShell 출력의 한글이 깨져 보인다(cp949). 필요하면 `[Console]::OutputEncoding`을 UTF-8로 바꾼다.
- bash에서 PowerShell 명령을 넘길 때 `$`를 이스케이프하거나 작은따옴표로 감싼다.
- PowerShell 함수 이름이 별칭과 충돌한다(`Mv`가 `Move-Item`). `Add-Type`의 C# 코드는 `System.Drawing` 참조를 명시한다.
- Rust: `~/.cargo/bin`이 PATH에 없다. `export PATH="$HOME/.cargo/bin:$PATH"`를 먼저 실행한다. Windows용 빌드는 링커가 없어 불가능하다.
- `sudo`는 비밀번호가 필요해서 에이전트가 쓸 수 없다. 필요하면 사용자에게 명령을 요청한다.
- 외부에서 받은 코드의 실행은 자동으로 차단된다. 사용자 승인이 필요하다. 클론은 `references/other-projects/`에 있고 gitignore이다.
- 임시 파일은 작업용 scratchpad 폴더에 둔다. 사용이 끝나면 지운다.