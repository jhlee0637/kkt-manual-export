#!/bin/bash
# Mac 시험·보고용 스크립트. 읽기만 하고 아무것도 바꾸지 않는다 (카카오톡 조작, 파일 수정 없음).
# 결과는 화면에 보여 주고 ~/Desktop/kkt-mac-report.txt 에도 저장한다. 보내기 전에 열어서 확인하라.
#
# 사용:  bash mac-report.sh [kkt 경로] [내보내기 TXT 경로]
#   kkt 경로 생략: 현재 폴더와 ~/Downloads 에서 kkt-macos-universal 을 찾는다.
#   TXT 경로 생략: ~/Downloads, ~/Documents, ~/Desktop 에서 가장 최근의 KakaoTalk*.txt 를 쓴다.
# 카카오톡 메뉴 구조는 "시스템 설정 > 개인정보 보호 및 보안 > 손쉬운 사용" 에서 터미널을 허용하면 읽힌다 (허용하지 않으면 그 사실만 보고).

set +e
OUT="$HOME/Desktop/kkt-mac-report.txt"
[ -d "$HOME/Desktop" ] || OUT="$HOME/kkt-mac-report.txt"
KKT="$1"
TXT="$2"
TMP="$(mktemp -d 2>/dev/null || echo /tmp/kkt-report-$$)"
mkdir -p "$TMP"
trap 'rm -rf "$TMP"' EXIT

exec > >(tee "$OUT") 2>&1

section() { printf '\n===== %s =====\n' "$1"; }
mask() { perl -CSDA -pe 's/\p{Hangul}/\x{AC00}/g; s/(?!\p{Hangul})[^\W\d_]/a/g' 2>/dev/null; }

echo "kkt Mac 보고서 ($(date '+%Y-%m-%d %H:%M:%S'))"

section "1. 환경"
sw_vers 2>&1
echo "arch: $(uname -m)"
echo "shell: $SHELL"

section "2. kkt 실행 파일"
if [ -z "$KKT" ]; then
  for c in ./kkt-macos-universal "$HOME/Downloads/kkt-macos-universal" ./kkt; do
    [ -f "$c" ] && KKT="$c" && break
  done
fi
if [ -z "$KKT" ] || [ ! -f "$KKT" ]; then
  echo "kkt 파일을 찾지 못했다 (경로를 첫 번째 인자로 주세요)"
else
  echo "경로: $KKT"
  ls -l "$KKT"
  shasum -a 256 "$KKT" 2>&1
  echo "-- 격리 표시(xattr):"; xattr -l "$KKT" 2>&1 | head -5; [ -z "$(xattr -l "$KKT" 2>/dev/null)" ] && echo "(없음)"
  echo "-- 서명(codesign):"; codesign -dv "$KKT" 2>&1 | head -6
  echo "-- 실행 가능 여부:"; [ -x "$KKT" ] && echo "실행 권한 있음" || echo "실행 권한 없음 (chmod +x 필요)"
  echo "-- --help:"; "$KKT" --help 2>&1 | head -3; echo "종료 코드: $?"
fi

section "3. 카카오톡"
APP=""
for a in /Applications/KakaoTalk.app "$HOME/Applications/KakaoTalk.app"; do [ -d "$a" ] && APP="$a" && break; done
if [ -z "$APP" ]; then
  echo "KakaoTalk.app 을 /Applications 에서 찾지 못했다"
else
  echo "경로: $APP"
  echo "버전: $(defaults read "$APP/Contents/Info" CFBundleShortVersionString 2>&1) (빌드 $(defaults read "$APP/Contents/Info" CFBundleVersion 2>&1))"
fi
pgrep -x KakaoTalk >/dev/null 2>&1 && echo "실행 중: 예" || echo "실행 중: 아니오 (켜 두면 아래 4번이 채워진다)"

section "4. 카카오톡 메뉴 구조와 단축키 (읽기 전용)"
if pgrep -x KakaoTalk >/dev/null 2>&1; then
  osascript <<'AS' 2>&1 | head -150
tell application "System Events"
  try
    tell process "KakaoTalk"
      set out to "창 수: " & (count of windows) & linefeed
      repeat with w in windows
        set out to out & "창: " & (name of w) & linefeed
      end repeat
      repeat with mb in menu bar items of menu bar 1
        set out to out & "[메뉴] " & (name of mb) & linefeed
        try
          repeat with mi in menu items of menu 1 of mb
            set nm to name of mi
            if nm is not missing value then
              set ck to ""
              try
                set ck to (value of attribute "AXMenuItemCmdChar" of mi)
                set md to (value of attribute "AXMenuItemCmdModifiers" of mi)
                set ck to " (" & ck & " / 수정키 " & md & ")"
              end try
              set out to out & "    " & nm & ck & linefeed
            end if
          end repeat
        end try
      end repeat
      return out
    end tell
  on error e
    return "읽지 못했다: " & e
  end try
end tell
AS
else
  echo "카카오톡이 실행 중이 아니라 건너뜀"
fi
echo "(창 이름에는 대화방 이름이 들어 있다. 공유하기 싫으면 이 줄을 지워도 된다)"

section "5. 내보내기 파일 형식"
if [ -z "$TXT" ]; then
  TXT="$(ls -t "$HOME"/Downloads/KakaoTalk*.txt "$HOME"/Documents/KakaoTalk*.txt "$HOME"/Desktop/KakaoTalk*.txt 2>/dev/null | head -1)"
fi
if [ -z "$TXT" ] || [ ! -f "$TXT" ]; then
  echo "내보내기 TXT 를 찾지 못했다. 카카오톡에서 대화를 내보낸 뒤 경로를 두 번째 인자로 주세요."
  echo "(내보내는 메뉴 경로와 기본 파일 이름도 알려 주세요)"
else
  echo "파일 이름: $(basename "$TXT")"
  echo "크기: $(wc -c < "$TXT") 바이트, 줄 수: $(wc -l < "$TXT")"
  echo "file 판정: $(file -b "$TXT")"
  echo "처음 4바이트(16진): $(head -c 4 "$TXT" | xxd -p)"
  echo "CRLF 줄 수: $(grep -c $'\r' "$TXT")"
  U8="$TMP/utf8.txt"
  if iconv -f UTF-8 -t UTF-8 "$TXT" >/dev/null 2>&1; then ENC="UTF-8"; cp "$TXT" "$U8"
  elif iconv -f UTF-16 -t UTF-8 "$TXT" > "$U8" 2>/dev/null && [ -s "$U8" ]; then ENC="UTF-16"
  elif iconv -f CP949 -t UTF-8 "$TXT" > "$U8" 2>/dev/null; then ENC="CP949"
  else ENC="알 수 없음"; : > "$U8"; fi
  echo "인코딩 판단: $ENC"
  echo "-- 처음 8줄 (한글은 '가', 영문은 'a' 로 가림. 숫자와 기호는 그대로):"
  head -8 "$U8" | tr -d '\r' | mask
  echo "-- 줄 모양 종류 (가린 모양 기준, 많이 나온 순, 상위 25개):"
  tr -d '\r' < "$U8" | mask | sed -E 's/[0-9]/9/g' | sort | uniq -c | sort -rn | head -25
  echo "-- 단어가 들어 있는 줄 수:"
  for k in 사진 이모티콘 삭제 들어왔 나갔 초대 오전 오후 AM PM; do
    printf '  %s: %s\n' "$k" "$(grep -c "$k" "$U8")"
  done
  echo "-- 처음 5개 날짜 줄(형식 확인용, 숫자만 남김):"
  grep -E '[0-9]{4}' "$U8" | tr -d '\r' | head -5 | mask

  section "6. kkt 로 정리해 보기 (임시 폴더, 끝나면 지움)"
  if [ -n "$KKT" ] && [ -x "$KKT" ]; then
    "$KKT" --archive "$TMP/archive" ingest "$TXT" 2>&1 | head -5
    echo "종료 코드: ${PIPESTATUS[0]}"
    echo "-- 한 번 더 (같은 파일은 건너뛰어야 함):"
    "$KKT" --archive "$TMP/archive" ingest "$TXT" 2>&1 | head -3
  else
    echo "kkt 를 실행할 수 없어 건너뜀"
  fi
fi

section "끝"
echo "보고서: $OUT"
echo "이 파일을 열어 개인정보(대화방 이름 등)를 확인한 뒤 보내 주세요. 원문 TXT 는 따로 보내 주세요."
