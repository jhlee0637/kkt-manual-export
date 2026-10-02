# 아카이브 스키마 (v2)

대화방 하나 = `archive/<conversation_id>/` 하나.

```
events.jsonl      추가 전용 이벤트 로그. 유일한 원본. 상태는 여기서 매번 복원한다.
raw/              반영한 내보내기 TXT 원본 (삭제하지 않는다)
attachments/image/<sha256 앞 2자>/<sha256>.<ext>
```

## 트랜잭션

이벤트는 묶음으로 쓰고 **종결 이벤트**로 닫는다 (`export.ingested`, `attach.committed`).
종결 이벤트 없이 끝난 꼬리(크래시)는 읽을 때 버리고 다음 쓰기 때 잘라낸다.

## 메시지 상태

| status | 의미 | 근거 |
|---|---|---|
| `active` | 마지막 내보내기에 보임 | |
| `deleted_for_everyone` | 보낸 사람이 모두에게 삭제 | 같은 자리에 `메시지가 삭제되었습니다.` 줄이 생김 (**확정**) |
| `missing` | 줄이 사라짐. **삭제 주체를 알 수 없다** | `나에게서만 삭제`는 흔적 없이 사라지므로 이 PC 사용자의 로컬 삭제일 수 있다 |
| `unverifiable` | 사라졌지만 새 내보내기의 날짜 범위 밖 | 내역이 잘렸을 가능성. 삭제로 보지 않는다 |

수정은 상태가 아니라 **버전**이다 (`version`, `history`). TXT에 수정 표시가 없어서 `edit_candidate`(후보)로만 기록한다.

## 이벤트

| type | 주요 필드 |
|---|---|
| `conversation.title_observed` / `conversation.renamed` | title / from, to, basis, link{matched, compared} |
| `participant.observed` | participant_id, name |
| `participant.renamed` | participant_id, from, to, matched_messages, evidence{messages, system_lines}, basis(`consistent_relabel`/`restored_previous_name`/`forced_by_user`) |
| `participant.linked` | participant_id(유지), merged_participant_id, current_name, basis=`manual` |
| `message.observed` | message_id, participant_id, date, hhmm, sender, text, content_type, content[], timestamp, timestamp_precision=`minute`, ordinal |
| `deleted_marker.observed` | 삭제 표식만 처음 보인 경우. 원래 내용은 알 수 없다. inferred_after/before |
| `system.observed` | 초대 등 시스템 줄 |
| `message.deleted_for_everyone` | message_id, inferred_after/before |
| `message.missing` | message_id, range_covered |
| `message.reappeared` | 사라졌던 같은 내용이 다시 보임 |
| `message.edit_candidate` | previous_text, current_text, confidence=`candidate` |
| `attachment.saved` | attachment_id, sha256, filename, aliases[], taken_at(초·밀리초), storage_key |
| `attachment.linked` | message_id, attachment_id, basis=`minute_match_ordered` |
| `export.ingested` | export_name, export_sha256, saved_at, visible[] (내보내기에 보인 id 순서), warnings |

모든 이벤트: `schema_version`, `event_id`(ev_000001…), `conversation_id`, `observed_at`.

## 시각

- TXT 시각은 **분 단위**다. `timestamp_precision: minute`로 표시한다. 초가 있는 것처럼 꾸미지 않는다.
- 사진은 저장 파일명(`KakaoTalk_YYYYMMDD_HHMMSSmmm`)에 초·밀리초가 있다 (`attachment.saved.taken_at`).
- 삭제 표식은 시각이 없다. 앞뒤 메시지로 범위만 추정한다 (`inferred_after`, `inferred_before`).

## 메시지 ID

카카오톡 TXT에는 ID가 없다. 첫 관측 때 `kmsg_<sha1(대화방,날짜,분,보낸이,내용,중복순번)>`를 부여하고,
이후 내보내기는 직전 내보내기와 **순서 기준으로 정렬**해서 같은 ID를 이어받는다.

## 사진 연결

같은 날짜·분 안에서 사진 메시지 수와 저장 파일 수가 **같을 때만** 시각 순서대로 짝짓는다.
다르면 연결하지 않고 `unmatched_groups`로 보고한다 (추측하지 않는다).
파일 내용이 같은 ` (1)` 중복 저장본은 `aliases`로 묶는다. 삭제된 메시지의 사진 파일도 보존한다.

## 방과 참가자의 정체성 (v2)

TXT에는 방 ID도 사용자 ID도 없다. 방 제목과 발신자 이름은 **식별자가 아니라 관측값**이다.

### 방 (`kkt/link.py`)
새 내보내기가 어느 방에 속하는지는 **제목 일치 + 메시지 겹침**(날짜·분·내용, 발신자 제외)으로 판정한다.

| 판정 | 조건 |
|---|---|
| `same` | 제목이 이미 본 제목이고 겹침 ≥ 50% (또는 비교 대상 3개 미만) |
| `renamed_room` | 제목은 처음이지만 한 방과 ≥ 60% 겹치고 다른 방은 < 30% → `conversation.renamed` 기록 |
| `new` | 어느 방과도 겹치지 않음 → 새 방 |
| **중단** | 같은 제목인데 내용이 다르거나, 처음 보는 제목인데 일부만 겹침 (잘못 합치면 되돌리기 어렵다) |

`conversation.renamed`에는 근거(`link`: 겹친 수/비교 수)가 남는다. 제목 이력은 `conversation.title_observed` / `renamed`.

**실측 (2026-10-02)**: 방 이름을 `test`→`test-2`로 바꾸자 내보내기 헤더가 `test-2 님과 카카오톡 대화`가 되고 채팅창 제목도 바뀌었다. 파일명 접미사(`_group`)와 창 핸들은 그대로였다. 겹침 10/10으로 `renamed_room` 판정됐다.

### 참가자
발신자는 이름이 아니라 `participant_id`(`kp_…`)로 비교한다. 한 참가자는 이름 이력(`names`)을 가지며, **옛 이름과 새 이름을 모두 같은 사람으로 알아본다**.

- **소급 변경** (이름 변경 뒤 내보내기가 과거 메시지에도 새 이름을 찍는 경우): 시각·내용이 같은 메시지끼리 맞췄을 때 이름만 일관되게 바뀌었으면 `participant.renamed` 로 **자동 확정**한다. 조건: 그 사람의 정렬된 메시지가 전부 같은 새 이름 하나로 바뀜 / 옛 이름이 새 내보내기에 더 남아 있지 않음 / 새 이름이 다른 참가자가 쓰던 이름이 아님 / 두 사람이 같은 새 이름으로 가지 않음 / 근거 2건 이상. 근거 = 이름만 달라진 메시지 수 + 같은 치환이 일어난 시스템 줄 수.
  - **실측 (2026-10-02)**: 이 PC 사용자가 **상대방의 이름을 직접 바꾸자**(로컬에서 이름 편집) 내보내기가 **과거 메시지의 발신자와 초대 시스템 줄 본문에도** 새 이름을 찍었다. 내보내기의 이름은 보낸 시점의 이름이 아니라 **내보내는 시점에 이 PC가 보여 주는 이름**이다. 메시지 근거 1건 + 시스템 줄 1건으로 자동 확정됐다. 이후 시스템 줄의 저장 본문도 새 이름으로 맞춘다 (이전 본문은 `history`).
  - **범위 밖 (결정)**: 상대방이 **자기 프로필 이름을 바꾼 경우**는 현재 고려하지 않는다. 관측하지 못했고(다른 계정이 필요), 운영 중에 실제로 발생하면 그때 고친다. 대비는 되어 있다: 소급이면 위와 같이 자동 확정되고, 새 메시지에만 나타나면 아래 비소급 경로(수동 연결)로 이어 붙인다 (둘 다 합성 데이터 테스트로만 검증).
  - **되돌림 (실측)**: 이름을 원래대로 되돌리자 내보내기가 과거 메시지와 시스템 줄에도 옛 이름을 되돌려 찍었다. 이 사람이 전에 쓰던 이름으로 통째로 바뀐 경우는 `participant.renamed`(`basis: restored_previous_name`)로 확정하고 현재 이름과 시스템 줄 본문을 되돌린다. 참가자는 그대로이고 `names` 이력에 두 이름이 모두 남는다.
  - 한 사람의 메시지가 **옛 이름과 새 이름으로 섞여 찍히는 것**(비소급)은 정상 상태이므로 경고도 이벤트도 내지 않는다. 경고는 이 사람이 모르는 이름이 나타났는데 확정하지 못한 경우에만 낸다.
  - 시스템 줄에서 이름은 `<이름>님` 꼴이므로 치환도 `님`까지 묶어서 한다 (`.` 같은 짧은 이름이 문장부호와 섞이지 않게).
- 근거가 약하면(1건 등) 확정하지 않고 경고를 남긴다. 사람이 확인하면 `ingest --accept-rename 옛이름=새이름` 으로 확정한다 (`basis: forced_by_user`).
- **비소급 변경** (과거 메시지는 옛 이름 그대로, 새 이름이 새 메시지에만 나타남): TXT 에 근거가 없다. 새 참가자로 기록되고, `participant-link --keep 옛이름 --merge 새이름` 으로 사람이 연결한다 (`participant.linked`, `basis: manual`).
- 같은 이름의 서로 다른 사람은 TXT 로 구분할 수 없다.

로그 버전이 오래되면(`schema_version` < 2) 조용히 잘못 읽지 않고 거부한다. 로그는 raw 내보내기에서 다시 만들 수 있다.

## 답장과 이모티콘 (2026-10-02 실측)

- **이모티콘**: TXT에 `이모티콘` 한 줄로 나온다 (`content_type: emoticon`). 어떤 이모티콘인지는 알 수 없다.
  글자 그대로 `이모티콘`이라고 보낸 메시지와는 TXT만으로 구분할 수 없다.
- **답장**: 화면에는 `○○에게 답장 / 인용문 / 본문`으로 보이지만, TXT에는 **본문만 일반 메시지로** 나온다.
  인용 정보가 없으므로 답장 관계는 TXT로 복원할 수 없다.
  모든 메시지의 `reply_to`는 `{"status": "unknown_from_txt"}`이다. 이는 "답장이 아니다"가 아니라 "알 수 없다"이다.
  답장 관계를 얻으려면 화면(캡처 + OCR)을 읽는 별도 수집 경로가 필요하다.

## 관측하지 못한 것 (추측하지 않고 text로 둔다)

동영상, 파일, 링크 미리보기의 TXT 형식. 다른 사람이 수정/삭제한 메시지의 표시.
사진 여러 장을 한 번에 보낸 경우의 줄 형식.
