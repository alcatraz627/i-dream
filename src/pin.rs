//! `i-dream pin <subcommand>` — session-pinned insights CLI per docs/18.
//!
//! Writes PinEvent JSON to ~/.claude/pinned/events.jsonl with flock + atomic
//! append. The `/pin-for-dream` skill shells out to `pin add --from-json -`
//! with a pre-composed event; humans can also use the bare CLI with flags.

use crate::cli::PinAction;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
struct PinEvent {
    id: String,
    ts: String,
    #[serde(default)]
    pinned_from: Option<PinnedFrom>,
    text: String,
    #[serde(default)]
    context: Option<PinContext>,
    #[serde(default = "default_framing")]
    framing: String,
    #[serde(default)]
    tool_signatures: Vec<String>,
    #[serde(default = "default_decay")]
    decay: PinDecay,
}

#[derive(Debug, Serialize, Deserialize)]
struct PinnedFrom {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PinContext {
    #[serde(default)]
    files: Vec<PinFile>,
    #[serde(default)]
    related_slugs: Vec<String>,
    #[serde(default)]
    related_paths_at_time: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PinFile {
    path: String,
    #[serde(default)]
    line_range: Option<[u32; 2]>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PinDecay {
    cycles_remaining: i64,
    #[serde(default)]
    first_seen_cycle: Option<String>,
    #[serde(default)]
    archived_at: Option<String>,
}

fn default_framing() -> String {
    "investigate".to_string()
}

fn default_decay() -> PinDecay {
    PinDecay {
        cycles_remaining: 2,
        first_seen_cycle: None,
        archived_at: None,
    }
}

pub fn handle(action: PinAction) -> Result<()> {
    match action {
        PinAction::Add {
            text,
            session_id,
            transcript,
            cwd,
            files,
            framing,
            tool_signatures,
            decay_cycles,
            from_json,
        } => add(
            text,
            session_id,
            transcript,
            cwd,
            files,
            framing,
            tool_signatures,
            decay_cycles,
            from_json,
        ),
        PinAction::List { include_archived } => list(include_archived),
        PinAction::Show { id } => show(&id),
        PinAction::Resolve { id } => resolve(&id),
        PinAction::Archived { since } => archived(since),
    }
}

#[allow(clippy::too_many_arguments)]
fn add(
    text: Option<String>,
    session_id: Option<String>,
    transcript: Option<String>,
    cwd: Option<String>,
    files: Vec<String>,
    framing: Option<String>,
    tool_signatures: Vec<String>,
    decay_cycles: u32,
    from_json: bool,
) -> Result<()> {
    let mut event = if from_json {
        // Read full PinEvent from stdin (skill mode).
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        let mut ev: PinEvent =
            serde_json::from_str(&buf).context("--from-json payload is not valid PinEvent")?;
        // Always regenerate id + ts to ensure they're fresh — caller may
        // have left them blank or stale.
        ev.ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        ev.id = mint_id(&ev.text, &ev.ts);
        ev
    } else {
        let text = text.context("text is required (or use --from-json -)")?;
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let id = mint_id(&text, &ts);
        let parsed_files = files
            .iter()
            .filter_map(|s| parse_file_arg(s))
            .collect::<Vec<_>>();
        PinEvent {
            id,
            ts,
            pinned_from: if session_id.is_some() || transcript.is_some() || cwd.is_some() {
                Some(PinnedFrom {
                    session_id,
                    transcript_path: transcript,
                    cwd,
                })
            } else {
                None
            },
            text,
            context: if parsed_files.is_empty() {
                None
            } else {
                Some(PinContext {
                    files: parsed_files,
                    related_slugs: vec![],
                    related_paths_at_time: vec![],
                })
            },
            framing: framing.unwrap_or_else(default_framing),
            tool_signatures,
            decay: PinDecay {
                cycles_remaining: decay_cycles as i64,
                first_seen_cycle: None,
                archived_at: None,
            },
        }
    };

    // Validate framing.
    if !matches!(
        event.framing.as_str(),
        "investigate" | "monitor" | "graduate" | "note"
    ) {
        bail!(
            "framing must be one of: investigate | monitor | graduate | note (got '{}')",
            event.framing
        );
    }

    let events_path = pinned_dir()?.join("events.jsonl");
    if let Some(parent) = events_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&events_path)?;

    // POSIX flock — best-effort; if libc::flock unavailable we still
    // succeed (single-writer is the common case).
    lock_file(&f)?;
    let line = serde_json::to_string(&event)?;
    writeln!(f, "{line}")?;
    f.flush()?;
    unlock_file(&f)?;

    println!("{}", event.id);
    Ok(())
}

fn list(include_archived: bool) -> Result<()> {
    let events = read_events()?;
    if events.is_empty() {
        println!("(no pins)");
        if include_archived {
            print_archived_brief()?;
        }
        return Ok(());
    }
    // Show newest first.
    let mut sorted = events;
    sorted.sort_by(|a, b| b.ts.cmp(&a.ts));
    println!("{:<28} {:<12} {}", "ID", "FRAMING", "TEXT");
    for e in &sorted {
        let truncated = e.text.replace('\n', " ");
        let truncated = if truncated.chars().count() > 60 {
            let mut s: String = truncated.chars().take(57).collect();
            s.push_str("…");
            s
        } else {
            truncated
        };
        println!("{:<28} {:<12} {}", e.id, e.framing, truncated);
    }
    if include_archived {
        print_archived_brief()?;
    }
    Ok(())
}

fn show(id: &str) -> Result<()> {
    let events = read_events()?;
    let Some(ev) = events.iter().find(|e| e.id == id) else {
        bail!("Pin '{id}' not found in events.jsonl (try `i-dream pin archived` for decayed ones)");
    };
    println!("{}", serde_json::to_string_pretty(ev)?);
    Ok(())
}

fn resolve(id: &str) -> Result<()> {
    // Validate the pin exists somewhere (active or already archived).
    let events = read_events()?;
    if !events.iter().any(|e| e.id == id) {
        bail!("Pin '{id}' not found");
    }

    // Append the resolve directive to _decay-state.json so the next
    // consolidate.sh run archives the pin.
    let state_path = pinned_dir()?.join("_decay-state.json");
    let mut state: serde_json::Map<String, Value> = if state_path.exists() {
        let s = fs::read_to_string(&state_path)?;
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    state.insert(id.to_string(), serde_json::json!(0));

    let tmp = state_path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&state)?)?;
    fs::rename(&tmp, &state_path)?;

    println!("Pin '{id}' marked for archival on next consolidate.sh run.");
    Ok(())
}

fn archived(since: Option<String>) -> Result<()> {
    let arch_root = pinned_dir()?.join("_archived");
    if !arch_root.exists() {
        println!("(nothing archived)");
        return Ok(());
    }
    let entries = fs::read_dir(&arch_root)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    let mut dates: Vec<&PathBuf> = entries.iter().collect();
    dates.sort();
    let since = since.unwrap_or_default();
    let mut shown = 0;
    for date_dir in dates {
        let name = date_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !since.is_empty() && name < since.as_str() {
            continue;
        }
        let jsonl = date_dir.join("events-decayed.jsonl");
        if !jsonl.exists() {
            continue;
        }
        println!("\n[{name}]");
        let f = fs::File::open(&jsonl)?;
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            if let Ok(ev) = serde_json::from_str::<PinEvent>(&line) {
                let preview = ev.text.replace('\n', " ");
                let preview = if preview.chars().count() > 70 {
                    let mut s: String = preview.chars().take(67).collect();
                    s.push_str("…");
                    s
                } else {
                    preview
                };
                println!("  {}  {}", ev.id, preview);
                shown += 1;
            }
        }
    }
    if shown == 0 {
        println!("(no archived pins matched)");
    }
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn pinned_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".claude/pinned"))
}

fn read_events() -> Result<Vec<PinEvent>> {
    let path = pinned_dir()?.join("events.jsonl");
    if !path.exists() {
        return Ok(vec![]);
    }
    let f = fs::File::open(&path)?;
    let mut out = vec![];
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<PinEvent>(&line) {
            out.push(ev);
        }
    }
    Ok(out)
}

fn print_archived_brief() -> Result<()> {
    let arch_root = pinned_dir()?.join("_archived");
    if !arch_root.exists() {
        return Ok(());
    }
    let count: usize = fs::read_dir(&arch_root)?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_dir() {
                fs::read_to_string(p.join("events-decayed.jsonl"))
                    .ok()
                    .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            } else {
                None
            }
        })
        .sum();
    println!("\n({count} archived — `i-dream pin archived`)");
    Ok(())
}

fn mint_id(text: &str, ts: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.update(ts.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    // Strip non-digit chars from ts for the id slug.
    let ts_compact: String = ts
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14) // YYYYMMDDHHMMSS
        .collect();
    format!("pin-{ts_compact}-{}", &hex[..2])
}

fn parse_file_arg(s: &str) -> Option<PinFile> {
    // Accepts "path" or "path:lineA-lineB"
    if let Some((path, range)) = s.rsplit_once(':') {
        if let Some((a, b)) = range.split_once('-')
            && let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
        {
            return Some(PinFile {
                path: path.to_string(),
                line_range: Some([a, b]),
            });
        }
    }
    Some(PinFile {
        path: s.to_string(),
        line_range: None,
    })
}

#[cfg(unix)]
fn lock_file(f: &fs::File) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let fd = f.as_raw_fd();
    // SAFETY: fd comes from a valid open File; LOCK_EX is a defined flag.
    let rc = unsafe { libc::flock(fd, libc::LOCK_EX) };
    if rc != 0 {
        bail!("flock failed: errno {}", std::io::Error::last_os_error());
    }
    // Re-seek to end after acquiring lock — other writers may have appended
    // since open. Use unsafe File::from_raw_fd-style mutability via libc lseek
    // to avoid needing &mut File at the call site.
    unsafe {
        libc::lseek(fd, 0, libc::SEEK_END);
    }
    Ok(())
}

#[cfg(unix)]
fn unlock_file(f: &fs::File) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let fd = f.as_raw_fd();
    let _ = unsafe { libc::flock(fd, libc::LOCK_UN) };
    Ok(())
}

#[cfg(not(unix))]
fn lock_file(_: &fs::File) -> Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn unlock_file(_: &fs::File) -> Result<()> {
    Ok(())
}
