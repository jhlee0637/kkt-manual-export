from kkt.archive import Archive
from kkt.attach import ingest_attachments, scan
from helpers import write_export


def png(path, payload: bytes):
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + payload)


def setup(tmp_path, image_minutes):
    arch = Archive(tmp_path / "a", "kt_test")
    lines = [f"[me] [오전 {m}] 사진" for m in image_minutes]
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 11:00:00", lines))
    src = tmp_path / "src"
    src.mkdir()
    return arch, src


def test_scan_ignores_foreign_files_and_dedups_copies(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    png(src / "KakaoTalk_20261002_100825860.png", b"A")
    png(src / "KakaoTalk_20261002_100825860 (1).png", b"A")      # 같은 내용의 중복 저장본
    (src / "report.pdf").write_bytes(b"%PDF user file")            # 사용자의 다른 파일
    files, skipped = scan(src)
    assert len(files) == 1 and files[0]["aliases"] == ["KakaoTalk_20261002_100825860 (1).png"]
    assert files[0]["taken_at"] == "2026-10-02T10:08:25.860+09:00"
    assert skipped == ["report.pdf"]


def test_links_when_counts_match_in_time_order(tmp_path):
    arch, src = setup(tmp_path, ["9:28", "9:28"])
    png(src / "KakaoTalk_20261002_092830000.png", b"second")
    png(src / "KakaoTalk_20261002_092801000.png", b"first")
    r = ingest_attachments(arch, src)
    assert r["saved"] == 2 and r["linked"] == 2 and r["unmatched_groups"] == []
    state, _ = arch.load()
    imgs = [x for x in state.registry.values() if x["content_type"] == "image"]
    taken = [state.attachments[x["attachment_id"]]["taken_at"] for x in imgs]
    assert taken == sorted(taken)                                  # 순서대로 대응
    for a in state.attachments.values():
        assert (arch.dir / a["storage_key"]).exists()


def test_count_mismatch_is_not_guessed(tmp_path):
    arch, src = setup(tmp_path, ["9:28", "9:28"])
    png(src / "KakaoTalk_20261002_092801000.png", b"only one")
    r = ingest_attachments(arch, src)
    assert r["saved"] == 1 and r["linked"] == 0
    assert r["unmatched_groups"] == [{"minute": "2026-10-02 09:28", "image_messages": 2, "files": 1}]


def test_second_run_is_noop_and_late_file_links_later(tmp_path):
    arch, src = setup(tmp_path, ["9:28"])
    r = ingest_attachments(arch, src)
    assert r == {"saved": 0, "linked": 0, "skipped_non_kakao_files": 0, "unmatched_groups": [
        {"minute": "2026-10-02 09:28", "image_messages": 1, "files": 0}]}
    png(src / "KakaoTalk_20261002_092800500.png", b"late")
    assert ingest_attachments(arch, src)["linked"] == 1
    assert ingest_attachments(arch, src)["saved"] == 0


def test_multi_photo_message_links_n_files_in_order(tmp_path):
    arch = Archive(tmp_path / "a", "kt_test")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 09:00:00", ["[me] [오전 8:07] 사진 3장", "[me] [오전 8:07] 사진"]))
    src = tmp_path / "src"
    src.mkdir()
    for i in range(4):
        png(src / f"KakaoTalk_20261002_0807{i:02d}000.png", f"p{i}".encode())
    r = ingest_attachments(arch, src)
    assert r["saved"] == 4 and r["linked"] == 4 and r["unmatched_groups"] == []
    state, _ = arch.load()
    first, second = [x for x in state.registry.values() if x["content_type"] == "image"]
    assert first["image_count"] == 3 and len(first["attachment_ids"]) == 3
    assert second["image_count"] == 1 and len(second["attachment_ids"]) == 1
    names = lambda r: [state.attachments[a]["filename"] for a in r["attachment_ids"]]
    assert names(first) == [f"KakaoTalk_20261002_0807{i:02d}000.png" for i in range(3)]      # 앞의 메시지가 앞의 파일 3장
    assert names(second) == ["KakaoTalk_20261002_080703000.png"]
    assert ingest_attachments(arch, src)["linked"] == 0                                        # 다시 해도 그대로


def test_multi_photo_count_mismatch_is_reported_not_guessed(tmp_path):
    arch = Archive(tmp_path / "a", "kt_test")
    arch.ingest(write_export(tmp_path, "e1.txt", "2026-10-02 09:00:00", ["[me] [오전 8:07] 사진 3장"]))
    src = tmp_path / "src"
    src.mkdir()
    png(src / "KakaoTalk_20261002_080700000.png", b"a")
    png(src / "KakaoTalk_20261002_080701000.png", b"b")
    r = ingest_attachments(arch, src)
    assert r["linked"] == 0
    assert r["unmatched_groups"] == [{"minute": "2026-10-02 08:07", "image_messages": 1, "files": 2, "photos": 3}]
