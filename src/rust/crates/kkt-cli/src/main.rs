//! kkt: 카카오톡 내보내기 아카이버 CLI (src/python/kkt/cli.py 의 이식).
//!
//! 규격은 tests/golden/README.md 의 "구현이 제공해야 하는 CLI" 를 따른다.
//!   전역 옵션: --archive <폴더>, --conversation <ID>
//!   서브커맨드: ingest, attach, status, participants, participant-link
//! 종료 코드: 성공 0, 처리 중단 1, 사용법 오류 2. 오류는 표준 오류에 `[중단:<code>] <메시지>` 로 낸다.

use indexmap::IndexMap;
use kkt_core::archive::{decode_export, Archive};
use kkt_core::attach::ingest_attachments;
use kkt_core::error::{KktError, Result};
use kkt_core::link::resolve;
use kkt_core::parse::{parse_export, ParsedExport};
use kkt_core::pyfmt::dumps;
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

mod guide;
mod ingest_flow;
mod media_plan;
mod screen;
mod settings;
mod wizard;

const USAGE: &str = "usage: kkt [--archive ARCHIVE] [--conversation CONVERSATION] {ingest,attach,status,participants,participant-link,collect,photos} ...";

struct Usage(String);

fn usage_err<T>(msg: impl Into<String>) -> std::result::Result<T, Usage> {
    Err(Usage(msg.into()))
}

fn take_value(args: &[String], i: &mut usize, flag: &str) -> std::result::Result<String, Usage> {
    *i += 1;
    match args.get(*i) {
        Some(v) => Ok(v.clone()),
        None => usage_err(format!("argument {flag}: expected one argument")),
    }
}

/// `--flag value` 와 `--flag=value` 를 모두 받는다.
fn opt(args: &[String], i: &mut usize, name: &str) -> std::result::Result<Option<String>, Usage> {
    let a = &args[*i];
    if a == name {
        return take_value(args, i, name).map(Some);
    }
    if let Some(v) = a.strip_prefix(&format!("{name}=")) {
        return Ok(Some(v.to_string()));
    }
    Ok(None)
}

enum Cmd {
    Ingest { files: Vec<String>, force: bool, accept: Vec<String> },
    Attach { src: String, videos: bool },
    Status,
    Participants,
    ParticipantLink { keep: String, merge: String },
    Collect { title: String, out: String, width: i32, height: i32, hold: f64, ingest: bool },
    Photos { title: String, newest: Option<usize>, expect_files: Option<usize>, save_dir: Option<String>, attach: bool, dry_run: bool, no_videos: bool },
}

struct Cli {
    archive: PathBuf,
    conversation: Option<String>,
    cmd: Cmd,
}

fn parse_args(args: &[String]) -> std::result::Result<Cli, Usage> {
    let mut archive = PathBuf::from("data/archive");
    let mut conversation: Option<String> = None;
    let mut i = 0;
    let sub = loop {
        let Some(a) = args.get(i) else { return usage_err("the following arguments are required: cmd") };
        if a == "-h" || a == "--help" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        if let Some(v) = opt(args, &mut i, "--archive")? {
            archive = PathBuf::from(v);
        } else if let Some(v) = opt(args, &mut i, "--conversation")? {
            conversation = Some(v);
        } else if a.starts_with('-') && a.len() > 1 {
            return usage_err(format!("unrecognized arguments: {a}"));
        } else {
            break a.clone();
        }
        i += 1;
    };
    let rest = &args[i + 1..];
    let cmd = match sub.as_str() {
        "ingest" => {
            let (mut files, mut force, mut accept) = (Vec::new(), false, Vec::new());
            let mut j = 0;
            while j < rest.len() {
                if rest[j] == "--force" {
                    force = true;
                } else if let Some(v) = opt(rest, &mut j, "--accept-rename")? {
                    accept.push(v);
                } else if rest[j].starts_with("--") {
                    return usage_err(format!("unrecognized arguments: {}", rest[j]));
                } else {
                    files.push(rest[j].clone());
                }
                j += 1;
            }
            if files.is_empty() {
                return usage_err("the following arguments are required: files");
            }
            Cmd::Ingest { files, force, accept }
        }
        "attach" => {
            let no_videos = rest.iter().any(|a| a == "--no-videos");
            let others: Vec<&String> = rest.iter().filter(|a| *a != "--no-videos").collect();
            match others.as_slice() {
                [src] if !src.starts_with("--") => Cmd::Attach { src: (*src).clone(), videos: !no_videos },
                _ => return usage_err("attach: expected exactly one argument: src"),
            }
        }
        "status" | "participants" => {
            if !rest.is_empty() {
                return usage_err(format!("unrecognized arguments: {}", rest.join(" ")));
            }
            if sub == "status" { Cmd::Status } else { Cmd::Participants }
        }
        "participant-link" => {
            let (mut keep, mut merge) = (None, None);
            let mut j = 0;
            while j < rest.len() {
                if let Some(v) = opt(rest, &mut j, "--keep")? {
                    keep = Some(v);
                } else if let Some(v) = opt(rest, &mut j, "--merge")? {
                    merge = Some(v);
                } else {
                    return usage_err(format!("unrecognized arguments: {}", rest[j]));
                }
                j += 1;
            }
            match (keep, merge) {
                (Some(keep), Some(merge)) => Cmd::ParticipantLink { keep, merge },
                _ => return usage_err("the following arguments are required: --keep, --merge"),
            }
        }
        "collect" => {
            let (mut title, mut out, mut width, mut height, mut hold, mut ingest) =
                (None, None, kkt_win::window::DEFAULT_WIDTH, kkt_win::window::DEFAULT_HEIGHT, 0.0, false);
            let mut j = 0;
            while j < rest.len() {
                if rest[j] == "--ingest" {
                    ingest = true;
                } else if let Some(v) = opt(rest, &mut j, "--title")? {
                    title = Some(v);
                } else if let Some(v) = opt(rest, &mut j, "--out")? {
                    out = Some(v);
                } else if let Some(v) = opt(rest, &mut j, "--width")? {
                    width = v.parse().or_else(|_| usage_err(format!("--width: invalid int value: {v:?}")))?;
                } else if let Some(v) = opt(rest, &mut j, "--height")? {
                    height = v.parse().or_else(|_| usage_err(format!("--height: invalid int value: {v:?}")))?;
                } else if let Some(v) = opt(rest, &mut j, "--hold")? {
                    hold = v.parse().or_else(|_| usage_err(format!("--hold: invalid float value: {v:?}")))?;
                } else {
                    return usage_err(format!("unrecognized arguments: {}", rest[j]));
                }
                j += 1;
            }
            match (title, out) {
                (Some(title), Some(out)) => Cmd::Collect { title, out, width, height, hold, ingest },
                _ => return usage_err("the following arguments are required: --title, --out"),
            }
        }
        "photos" => {
            let (mut title, mut newest, mut expect_files, mut save_dir, mut attach) = (None, None, None, None, false);
            let (mut dry_run, mut no_videos) = (false, false);
            let mut j = 0;
            while j < rest.len() {
                if rest[j] == "--attach" {
                    attach = true;
                } else if rest[j] == "--dry-run" {
                    dry_run = true;
                } else if rest[j] == "--no-videos" {
                    no_videos = true;
                } else if let Some(v) = opt(rest, &mut j, "--title")? {
                    title = Some(v);
                } else if let Some(v) = opt(rest, &mut j, "--newest")? {
                    newest = Some(v.parse().or_else(|_| usage_err(format!("--newest: invalid int value: {v:?}")))?);
                } else if let Some(v) = opt(rest, &mut j, "--expect-files")? {
                    expect_files = Some(v.parse().or_else(|_| usage_err(format!("--expect-files: invalid int value: {v:?}")))?);
                } else if let Some(v) = opt(rest, &mut j, "--save-dir")? {
                    save_dir = Some(v);
                } else {
                    return usage_err(format!("unrecognized arguments: {}", rest[j]));
                }
                j += 1;
            }
            match title {
                Some(title) => Cmd::Photos { title, newest, expect_files, save_dir, attach, dry_run, no_videos },
                None => return usage_err("the following arguments are required: --title"),
            }
        }
        other => return usage_err(format!("argument cmd: invalid choice: {other:?}")),
    };
    Ok(Cli { archive, conversation, cmd })
}

fn arch(cli: &Cli) -> Result<Archive> {
    match cli.conversation.as_deref() {
        Some(c) if !c.is_empty() => Ok(Archive::new(&cli.archive, c)),
        _ => Err(KktError::new("usage", "--conversation 이 필요하다")),
    }
}

fn parse_accept(values: &[String]) -> Result<IndexMap<String, String>> {
    let mut out = IndexMap::new();
    for v in values {
        match v.split_once('=') {
            Some((o, n)) if !o.is_empty() && !n.is_empty() => {
                out.insert(o.to_string(), n.to_string());
            }
            _ => return Err(KktError::new("usage", format!("--accept-rename 형식은 '옛이름=새이름' 이다: {v:?}"))),
        }
    }
    Ok(out)
}

fn print_line(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", dumps(v));
    let _ = out.flush();
}

fn print_pretty(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).expect("직렬화"));
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn cmd_ingest(cli: &Cli, files: &[String], force: bool, accept: &[String]) -> Result<()> {
    let mut items: Vec<(String, PathBuf, ParsedExport)> = Vec::new();
    for f in files {
        let p = PathBuf::from(f);
        let parsed = parse_export(&decode_export(&fs::read(&p)?)?);
        items.push((parsed.saved_at.clone().unwrap_or_default(), p, parsed));
    }
    items.sort_by(|a, b| (&a.0, file_name(&a.1)).cmp(&(&b.0, file_name(&b.1)))); // 시간순으로 반영해야 삭제/수정 판정이 맞다
    let accept = parse_accept(accept)?;
    for (_, p, parsed) in &items {
        print_line(&ingest_parsed(&cli.archive, cli.conversation.as_deref(), p, parsed, force, &accept)?);
    }
    Ok(())
}

fn ingest_parsed(archive: &Path, conversation: Option<&str>, p: &Path, parsed: &ParsedExport, force: bool, accept: &IndexMap<String, String>) -> Result<Value> {
    // 파일마다 방을 판정한다. 중간에 방 이름이 바뀐 내보내기가 섞여 있어도 이어 붙는다.
    let (conv, link) = resolve(archive, parsed, conversation, force)?;
    let r = Archive::new(archive, &conv).ingest(p, Some(parsed), force, accept, Some(&link))?;
    let mut m = Map::new();
    m.insert("conversation".into(), conv.into());
    m.insert("link".into(), link["status"].clone());
    if let Value::Object(rm) = r {
        m.extend(rm);
    }
    Ok(Value::Object(m))
}

fn count_by<F: Fn(&kkt_core::state::Record) -> Option<String>>(st: &kkt_core::state::State, f: F) -> Map<String, Value> {
    let mut c: IndexMap<String, u64> = IndexMap::new();
    for r in st.registry.values() {
        if let Some(k) = f(r) {
            *c.entry(k).or_insert(0) += 1;
        }
    }
    c.into_iter().map(|(k, v)| (k, Value::from(v))).collect()
}

fn cmd_status(cli: &Cli) -> Result<()> {
    let a = arch(cli)?;
    let (st, _) = a.load()?;
    let msgs: Vec<_> = st.registry.values().filter(|r| r.kind == "message").collect();
    let mut status = json!({
        "conversation_id": a.conversation_id,
        "title": st.title,
        "participants": st.participants.len(),
        "exports_ingested": st.ingests.len(),
        "last_export_saved_at": st.ingests.last().and_then(|i| i.saved_at.clone()),
        "messages_by_status": count_by(&st, |r| (r.kind == "message").then(|| r.status.clone())),
        "image_messages": msgs.iter().filter(|r| r.content_type == "image").count(),
        "image_messages_linked": msgs.iter().filter(|r| r.content_type == "image" && r.attachment_ids.len() as u64 >= r.image_count).count(),
        "deleted_markers_unmatched": st.registry.values().filter(|r| r.kind == "deleted_marker").count(),
        "attachments": st.attachments.len(),
    });
    if let Some(m) = status.as_object_mut() {
        let videos: Vec<_> = msgs.iter().filter(|r| r.content_type == "video").collect();
        if !videos.is_empty() {
            // 동영상 메시지가 있을 때만 나타나는 키 (기존 출력은 그대로)
            m.insert("video_messages".into(), videos.len().into());
            m.insert("video_messages_linked".into(), videos.iter().filter(|r| !r.attachment_ids.is_empty()).count().into());
        }
    }
    print_pretty(&status);
    Ok(())
}

fn cmd_participants(cli: &Cli) -> Result<()> {
    let (st, _) = arch(cli)?.load()?;
    let parts: Vec<Value> = st
        .participants
        .values()
        .map(|p| {
            json!({"id": p.id, "current": p.current, "names": p.names,
                   "messages": st.registry.values().filter(|r| r.participant_id.as_deref() == Some(p.id.as_str())).count()})
        })
        .collect();
    print_pretty(&json!({"title": st.title, "titles": st.titles, "participants": parts}));
    Ok(())
}

/// 카카오톡 창을 조작해 내보내기 TXT 를 저장한다 (Windows 전용). 종료 코드: 성공 0, 실패 1, Ctrl+D 중단 130.
fn cmd_collect(cli: &Cli, title: &str, out: &str, width: i32, height: i32, hold: f64, ingest: bool) -> i32 {
    use kkt_win::{collect, guard::Guard, WinError};
    let opt = collect::Options { width, height, hold, ..Default::default() };
    let meta = match collect::export_chat(title, Path::new(out), &Guard::new(), &opt) {
        Ok(m) => m,
        Err(WinError::Aborted) => {
            eprintln!("[중단] Ctrl+D 로 중단됨");
            return 130;
        }
        Err(e) => {
            eprintln!("[실패] {e}");
            return 1;
        }
    };
    print_line(&meta);
    if ingest {
        let path = meta["path"].as_str().unwrap_or_default().to_string();
        if let Err(e) = cmd_ingest(cli, &[path], false, &[]) {
            eprintln!("[중단:{}] {}", e.code, e.message);
            return 1;
        }
    }
    0
}

/// 서랍의 사진을 저장한다 (Windows 전용). `--attach` 면 저장 폴더를 읽어 메시지와 연결한다.
fn cmd_photos(cli: &Cli, title: &str, newest: Option<usize>, expect_files: Option<usize>, save_dir: &Option<String>, attach: bool, dry_run: bool, no_videos: bool) -> i32 {
    use kkt_win::{guard::Guard, photos, WinError};
    let dir = save_dir.as_ref().map(PathBuf::from).unwrap_or_else(photos::default_save_dir);
    let opt = photos::Options { newest, expect_files, skip_videos: no_videos, dry_run, ..Default::default() };
    let meta = match photos::download_photos(title, &dir, &Guard::new(), &opt) {
        Ok(m) => m,
        Err(WinError::Aborted) => {
            eprintln!("[중단] Ctrl+D 로 중단됨");
            return 130;
        }
        Err(e) => {
            eprintln!("[실패] {e}");
            return 1;
        }
    };
    print_line(&meta);
    if attach {
        let result = arch(cli).and_then(|a| ingest_attachments(&a, &dir, !no_videos));
        match result {
            Ok(v) => print_pretty(&v),
            Err(e) => {
                eprintln!("[중단:{}] {}", e.code, e.message);
                return 1;
            }
        }
    }
    0
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Ingest { files, force, accept } => cmd_ingest(cli, files, *force, accept),
        Cmd::Attach { src, videos } => {
            print_pretty(&ingest_attachments(&arch(cli)?, Path::new(src), *videos)?);
            Ok(())
        }
        Cmd::Collect { .. } | Cmd::Photos { .. } => unreachable!("main 에서 처리한다"),
        Cmd::Status => cmd_status(cli),
        Cmd::Participants => cmd_participants(cli),
        Cmd::ParticipantLink { keep, merge } => {
            print_pretty(&arch(cli)?.link_participants(keep, merge)?);
            Ok(())
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // 인자 없이 실행(더블클릭)하면 안내 마당을 연다. `kkt wizard` 로도 연다.
    if args.is_empty() || args == ["wizard"] {
        std::process::exit(wizard::run());
    }
    let cli = match parse_args(&args) {
        Ok(c) => c,
        Err(Usage(msg)) => {
            eprintln!("{USAGE}\nkkt: error: {msg}");
            std::process::exit(2);
        }
    };
    if let Cmd::Collect { title, out, width, height, hold, ingest } = &cli.cmd {
        std::process::exit(cmd_collect(&cli, title, out, *width, *height, *hold, *ingest));
    }
    if let Cmd::Photos { title, newest, expect_files, save_dir, attach, dry_run, no_videos } = &cli.cmd {
        std::process::exit(cmd_photos(&cli, title, *newest, *expect_files, save_dir, *attach, *dry_run, *no_videos));
    }
    if let Err(e) = run(&cli) {
        eprintln!("[중단:{}] {}", e.code, e.message);
        std::process::exit(1);
    }
}
