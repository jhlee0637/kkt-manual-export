# AGENTS.md
이 폴더는 카카오톡 대화 기록 보관소입니다.
- `archive{SEP}<폴더>{SEP}events.jsonl`이 유일한 원본입니다. 추가만 되는 기록이므로 수정·삭제하지 마세요.
- 채팅방 이름과 폴더의 대응은 `README.md`의 채팅방 목록을 보세요.
- 메시지 상태는 `active`, `deleted_for_everyone`, `missing`, `unverifiable`입니다. 수정은 상태가 아니라 버전입니다.
- 사진·동영상은 `attachments{SEP}`에 있고 `attachment.linked` 이벤트로 메시지와 이어집니다.
- 대화 원문·사진·실명은 외부 서비스에 올리지 마세요.
- 형식 전체는 프로그램 저장소의 `docs/SCHEMA.md`를 보세요.
