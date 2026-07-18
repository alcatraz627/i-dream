//! The `status` verb — one report answering "is the subconscious healthy,
//! and if not, what exactly is wrong?"
//!
//! Gathering and rendering are separated: `gather()` reads the filesystem
//! into a serializable `StatusReport`; the renderers only format it. Plain
//! `status` stays instant (PID file, state.json, lane stats). The deep
//! sections — scheduled-job fires, log noise, binary freshness — spawn
//! `launchctl`/`ps` and scan a log file, so they are gathered only for
//! `--verbose` / `--json`.
//!
//! Lane health is computed live from the filesystem (same stat calls the
//! daemon makes each cycle) rather than read back from lane-health.jsonl,
//! so the verdict can never be a cycle stale.

use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::daemon::{DaemonState, is_process_alive, pid_file_path, read_pid_file};
use crate::modules::registry::{self, LaneHealth, LaneStatus};

#[derive(Serialize)]
pub struct StatusReport {
    pub daemon: DaemonSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<DaemonState>,
    /// Present when state.json exists but can't be parsed — status must
    /// report corruption, not die on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_error: Option<String>,
    pub lanes: LanesSection,
    pub queue: QueueSection,
    pub modules: Vec<ModuleInit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jobs: Option<Vec<JobStatus>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<LogSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildSection>,
}

#[derive(Serialize)]
pub struct DaemonSection {
    /// "running" | "stopped" | "stale-pid-file"
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    /// Present on a stale-pid-file verdict — names the file to clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid_file: Option<String>,
}

#[derive(Serialize)]
pub struct LanesSection {
    pub green: usize,
    pub yellow: usize,
    pub red: usize,
    pub lanes: Vec<LaneHealth>,
}

#[derive(Serialize)]
pub struct QueueSection {
    /// Entries currently sitting in dreams/ingest-queue.
    pub depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest: Option<String>,
}

#[derive(Serialize)]
pub struct ModuleInit {
    pub name: &'static str,
    pub initialized: bool,
}

#[derive(Serialize)]
pub struct JobStatus {
    pub label: &'static str,
    pub desc: &'static str,
    pub schedule: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fire: Option<String>,
    pub installed: bool,
    pub loaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_exit: Option<i64>,
    /// The binary launchd reports for this label. launchd is global per
    /// user, so under a sandboxed $HOME a planted plist with a live label
    /// reads the REAL session's job — this field makes such a collision
    /// visible instead of silent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
}

#[derive(Serialize)]
pub struct LogSection {
    pub file: String,
    pub warn_count: u64,
    pub error_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Serialize)]
pub struct BuildSection {
    pub version: &'static str,
    /// The binary answering this status call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_modified: Option<DateTime<Utc>>,
    /// The binary the daemon is actually running (from `ps`) — can differ
    /// from `binary` when status runs from a dev build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_exe: Option<String>,
    /// False when the PID in the pid file resolves to a process that is
    /// not i-dream at all (stale or reused PID) — freshness verdicts are
    /// withheld in that case rather than confidently wrong.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_is_i_dream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_exe_modified: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_started: Option<DateTime<Utc>>,
    /// True when the daemon started before its own binary was last
    /// written — the file was replaced and the daemon runs old code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest_source_change: Option<DateTime<Utc>>,
    /// True when a source file is newer than the deployed binary (the
    /// daemon's when running, else this one) — a rebuild is due.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_behind_source: Option<bool>,
}

/// Read everything the report needs. `deep` adds the sections that spawn
/// subprocesses or scan logs (verbose / JSON mode).
pub fn gather(deep: bool) -> Result<StatusReport> {
    let home = dirs::home_dir().unwrap_or_default();
    let data_dir = home.join(".claude/subconscious");

    // Daemon liveness — verify the PID, don't trust the file alone.
    let pid_path = pid_file_path();
    let daemon = match read_pid_file(&pid_path) {
        Some(pid) if is_process_alive(pid) => DaemonSection {
            status: "running".into(),
            pid: Some(pid),
            pid_file: None,
        },
        Some(pid) => DaemonSection {
            status: "stale-pid-file".into(),
            pid: Some(pid),
            pid_file: Some(pid_path.display().to_string()),
        },
        None => DaemonSection {
            status: "stopped".into(),
            pid: None,
            pid_file: None,
        },
    };

    // state.json — corruption is a finding, not a crash.
    let (state, state_error) = match std::fs::read_to_string(data_dir.join("state.json")) {
        Ok(content) => match serde_json::from_str::<DaemonState>(&content) {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(format!("state.json unparseable: {e}"))),
        },
        Err(_) => (None, None),
    };

    // Lane health — measured live, never a cycle stale.
    let lane_rows = registry::compute_lane_health(&home);
    let (mut green, mut yellow, mut red) = (0, 0, 0);
    for l in &lane_rows {
        match l.status {
            LaneStatus::Green => green += 1,
            LaneStatus::Yellow => yellow += 1,
            LaneStatus::Red => red += 1,
        }
    }
    let lanes = LanesSection {
        green,
        yellow,
        red,
        lanes: lane_rows,
    };

    // Ingest queue — depth plus the age of the oldest unconsumed item.
    let queue_dir = data_dir.join("dreams/ingest-queue");
    let depth = std::fs::read_dir(&queue_dir)
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    let queue = QueueSection {
        depth,
        oldest: registry::oldest_child_age(&queue_dir).map(registry::fmt_age),
    };

    let modules = ["dreams", "metacog", "valence", "introspection", "intentions"]
        .iter()
        .map(|name| ModuleInit {
            name,
            initialized: data_dir.join(name).exists(),
        })
        .collect();

    let (jobs, log, build) = if deep {
        (
            Some(gather_jobs()),
            gather_log(&data_dir.join("logs")),
            Some(gather_build(daemon.pid)),
        )
    } else {
        (None, None, None)
    };

    Ok(StatusReport {
        daemon,
        state,
        state_error,
        lanes,
        queue,
        modules,
        jobs,
        log,
        build,
    })
}

/// One-line lane summary: counts plus every non-green lane named with its
/// reason, so plain `status` answers "is anything broken?" by itself.
pub fn lanes_summary_line(lanes: &LanesSection) -> String {
    let mut line = format!("Lanes: {} green", lanes.green);
    if lanes.yellow > 0 {
        line.push_str(&format!(" · {} yellow", lanes.yellow));
    }
    if lanes.red > 0 {
        line.push_str(&format!(" · {} red", lanes.red));
    }
    let attention: Vec<String> = lanes
        .lanes
        .iter()
        .filter(|l| l.status == LaneStatus::Red)
        .chain(lanes.lanes.iter().filter(|l| l.status == LaneStatus::Yellow))
        .map(|l| format!("{}: {}", l.lane, l.reason))
        .collect();
    if !attention.is_empty() {
        line.push_str(&format!(" ({})", attention.join(" · ")));
    }
    line
}

fn lane_word(s: LaneStatus) -> &'static str {
    match s {
        LaneStatus::Green => "green",
        LaneStatus::Yellow => "yellow",
        LaneStatus::Red => "red",
    }
}

// ── deep gatherers ───────────────────────────────────────────────────────────

/// Scheduled-job rows: schedule + next local fire, plus launchd's view when
/// the plist is installed under this $HOME. Caveat: launchd itself is
/// global per user — it does not key by $HOME. The plist-existence gate
/// keeps a fresh sandbox from querying it at all, but a sandbox that plants
/// a plist whose label is live in the real session will read the REAL job's
/// state; the `program` field is carried so that collision is visible.
fn gather_jobs() -> Vec<JobStatus> {
    let now = Local::now();
    crate::cron::JOBS
        .iter()
        .map(|job| {
            let installed = crate::cron::plist_path(job.label)
                .map(|p| p.exists())
                .unwrap_or(false);
            let (loaded, pid, last_exit, program) = if installed {
                match std::process::Command::new("launchctl")
                    .args(["list", job.label])
                    .output()
                {
                    Ok(out) if out.status.success() => {
                        let (pid, last_exit, program) =
                            parse_launchctl_list(&String::from_utf8_lossy(&out.stdout));
                        (true, pid, last_exit, program)
                    }
                    _ => (false, None, None, None),
                }
            } else {
                (false, None, None, None)
            };
            JobStatus {
                label: job.label,
                desc: job.desc,
                schedule: job.schedule.human(),
                next_fire: job
                    .schedule
                    .next_fire_after(now)
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                installed,
                loaded,
                pid,
                last_exit,
                program,
            }
        })
        .collect()
}

/// Extract `"PID" = N;`, `"LastExitStatus" = N;`, and `"Program" = "...";`
/// from `launchctl list <label>` output. A missing PID just means the job
/// isn't running right now. PIDs are real process ids — non-positive values
/// are treated as absent rather than passed through.
fn parse_launchctl_list(out: &str) -> (Option<i64>, Option<i64>, Option<String>) {
    let line_value = |key: &str| {
        out.lines()
            .find(|l| l.contains(&format!("\"{key}\"")))
            .and_then(|l| l.split('=').nth(1))
            .map(|v| v.trim().trim_end_matches(';').trim().to_string())
    };
    let grab_int = |key: &str| line_value(key).and_then(|v| v.parse::<i64>().ok());
    (
        grab_int("PID").filter(|pid| *pid > 0),
        grab_int("LastExitStatus"),
        line_value("Program").map(|v| v.trim_matches('"').to_string()),
    )
}

/// Count WARN/ERROR lines in the newest rolling log file. Daily files stay
/// in the tens of KB, so a full read is fine.
fn gather_log(logs_dir: &Path) -> Option<LogSection> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(logs_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("i-dream.log."))
        })
        .collect();
    candidates.sort(); // date-suffixed names sort chronologically
    let newest = candidates.pop()?;
    let content = std::fs::read_to_string(&newest).ok()?;
    let (warn_count, error_count, last_error) = scan_log(&content);
    Some(LogSection {
        file: newest.file_name()?.to_string_lossy().into_owned(),
        warn_count,
        error_count,
        last_error,
    })
}

/// Count tracing-formatted WARN/ERROR lines and keep the last ERROR line.
fn scan_log(content: &str) -> (u64, u64, Option<String>) {
    let mut warns = 0;
    let mut errors = 0;
    let mut last_error = None;
    for line in content.lines() {
        if line.contains(" WARN ") {
            warns += 1;
        } else if line.contains(" ERROR ") {
            errors += 1;
            last_error = Some(line.to_string());
        }
    }
    (warns, errors, last_error)
}

/// Binary/daemon freshness — the checks that catch "the fix is committed
/// but nothing is running it". Both compare against the binary that is
/// actually deployed: the daemon's own executable when it runs (statused
/// via `ps`, so a dev-build `status` can't misattribute its freshness),
/// falling back to this process's binary when the daemon is down.
fn gather_build(daemon_pid: Option<i32>) -> BuildSection {
    let mtime_utc = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Utc>::from)
    };

    let binary_path = std::env::current_exe().ok();
    let binary_modified = binary_path.as_deref().and_then(mtime_utc);

    let daemon_exe = daemon_pid.and_then(process_exe_path);
    let daemon_exe_modified = daemon_exe.as_deref().and_then(mtime_utc);
    // Liveness (kill -0) says nothing about identity: a recycled PID can
    // belong to any process. If the exe isn't i-dream, say so and withhold
    // freshness verdicts instead of judging a stranger's binary.
    let daemon_is_i_dream = daemon_exe
        .as_deref()
        .map(|p| p.file_name().is_some_and(|n| n == "i-dream"));

    let daemon_started = daemon_pid
        .and_then(process_start_time)
        .map(|t| t.with_timezone(&Utc));

    // CARGO_MANIFEST_DIR is baked at compile time — valid on the machine
    // the binary was built on, silently absent anywhere else.
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let newest_source_change = newest_rs_mtime(&src_dir).map(DateTime::<Utc>::from);

    let (daemon_stale, binary_behind_source) = freshness_verdicts(
        daemon_started,
        daemon_is_i_dream.unwrap_or(false),
        daemon_exe_modified,
        binary_modified,
        newest_source_change,
    );

    BuildSection {
        version: env!("CARGO_PKG_VERSION"),
        binary: binary_path.map(|p| p.display().to_string()),
        binary_modified,
        daemon_exe: daemon_exe.map(|p| p.display().to_string()),
        daemon_is_i_dream,
        daemon_exe_modified,
        daemon_started,
        daemon_stale,
        newest_source_change,
        binary_behind_source,
    }
}

/// Pure freshness comparison, split from the gatherers so tests can pin
/// each verdict's SOURCE: staleness compares the daemon's own exe (never
/// the status process's binary — they differ under a dev build), and a
/// PID that isn't running i-dream gets no verdict at all rather than a
/// confident wrong one. Returns (daemon_stale, binary_behind_source).
fn freshness_verdicts(
    daemon_started: Option<DateTime<Utc>>,
    daemon_exe_is_idream: bool,
    daemon_exe_modified: Option<DateTime<Utc>>,
    binary_modified: Option<DateTime<Utc>>,
    newest_source_change: Option<DateTime<Utc>>,
) -> (Option<bool>, Option<bool>) {
    let daemon_stale = match (daemon_started, daemon_exe_modified) {
        // `ps lstart` is second-granular while mtimes are not, so a daemon
        // respawned within the same second its binary landed (every
        // install.sh run) would read as stale. Require the binary to be a
        // full second newer than the start before calling it that.
        (Some(started), Some(built)) if daemon_exe_is_idream => {
            Some(built > started + chrono::Duration::seconds(1))
        }
        _ => None,
    };
    // The deployed binary: the daemon's exe when a real daemon runs, else
    // whatever binary answered this call.
    let deployed_modified = if daemon_exe_is_idream {
        daemon_exe_modified.or(binary_modified)
    } else {
        binary_modified
    };
    let binary_behind_source = match (newest_source_change, deployed_modified) {
        (Some(src), Some(bin)) => Some(src > bin),
        _ => None,
    };
    (daemon_stale, binary_behind_source)
}

/// Executable path of a live process via `ps -o comm=`. On macOS this is
/// the full path launchd/exec recorded, which is exactly what we want to
/// stat for replaced-after-start detection.
pub(crate) fn process_exe_path(pid: i32) -> Option<PathBuf> {
    let out = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(PathBuf::from(s)) }
}

/// Newest mtime among `.rs` files under `dir`, recursively.
fn newest_rs_mtime(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let candidate = if path.is_dir() {
            newest_rs_mtime(&path)
        } else if path.extension().is_some_and(|e| e == "rs") {
            entry.metadata().ok().and_then(|m| m.modified().ok())
        } else {
            None
        };
        if let Some(t) = candidate
            && newest.is_none_or(|n| t > n)
        {
            newest = Some(t);
        }
    }
    newest
}

/// Start time of a live process via `ps -o lstart=`. None when the process
/// is gone or the output doesn't parse.
fn process_start_time(pid: i32) -> Option<DateTime<Local>> {
    let out = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lstart(&String::from_utf8_lossy(&out.stdout))
}

/// Parse BSD `ps -o lstart=` output ("Thu Jul 17 11:50:23 2026").
/// Tokenized first so single-digit days (double space) parse too.
fn parse_lstart(s: &str) -> Option<DateTime<Local>> {
    use chrono::TimeZone;
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let [_wday, mon, day, time, year] = tokens.as_slice() else {
        return None;
    };
    let naive = chrono::NaiveDateTime::parse_from_str(
        &format!("{mon} {day} {time} {year}"),
        "%b %d %H:%M:%S %Y",
    )
    .ok()?;
    chrono::Local.from_local_datetime(&naive).earliest()
}

// ── rendering ────────────────────────────────────────────────────────────────

pub fn render_text(r: &StatusReport, verbose: bool) -> String {
    let mut out = String::new();

    match (&r.daemon.status[..], r.daemon.pid) {
        ("running", Some(pid)) => out.push_str(&format!("Daemon: running (PID {pid})\n")),
        ("stale-pid-file", Some(pid)) => {
            let at = r
                .daemon
                .pid_file
                .as_deref()
                .map(|p| format!(" at {p}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "Daemon: stopped (stale PID file{at}, PID {pid} is not alive)\n"
            ));
        }
        _ => out.push_str("Daemon: stopped\n"),
    }

    if let Some(state) = &r.state {
        if let Some(last) = state.last_consolidation {
            out.push_str(&format!("Last consolidation: {last}\n"));
        }
        if let Some(activity) = state.last_activity {
            out.push_str(&format!("Last activity: {activity}\n"));
        }
        out.push_str(&format!("Total cycles: {}\n", state.total_cycles));
        out.push_str(&format!("Total tokens used: {}\n", state.total_tokens_used));
    }
    if let Some(err) = &r.state_error {
        out.push_str(&format!("⚠ {err}\n"));
    }

    out.push_str(&lanes_summary_line(&r.lanes));
    out.push('\n');
    if r.queue.depth > 0 {
        out.push_str(&format!(
            "Queue: {} pending{}\n",
            r.queue.depth,
            r.queue
                .oldest
                .as_deref()
                .map(|o| format!(" · oldest {o}"))
                .unwrap_or_default()
        ));
    }

    out.push_str("\nModules:\n");
    for m in &r.modules {
        let word = if m.initialized {
            "initialized"
        } else {
            "not initialized"
        };
        out.push_str(&format!("  {}: {}\n", m.name, word));
    }

    if verbose {
        render_verbose(r, &mut out);
    }
    out
}

fn render_verbose(r: &StatusReport, out: &mut String) {
    // Full lane table — status, lane, reason, consumer.
    out.push_str(&format!("\nLanes ({}):\n", r.lanes.lanes.len()));
    let name_w = r
        .lanes
        .lanes
        .iter()
        .map(|l| l.lane.len())
        .max()
        .unwrap_or(0);
    let reason_w = r
        .lanes
        .lanes
        .iter()
        .map(|l| l.reason.len())
        .max()
        .unwrap_or(0);
    for l in &r.lanes.lanes {
        out.push_str(&format!(
            "  {:6} {:name_w$}  {:reason_w$}  → {}\n",
            lane_word(l.status),
            l.lane,
            l.reason,
            l.consumer,
        ));
    }

    if let Some(jobs) = &r.jobs {
        out.push_str("\nScheduled jobs:\n");
        for j in jobs {
            let mut line = format!("  {}  {}", j.label, j.schedule);
            if let Some(next) = &j.next_fire {
                line.push_str(&format!(" · next {next}"));
            }
            if !j.installed {
                line.push_str(" · NOT INSTALLED (i-dream cron install)");
            } else if !j.loaded {
                line.push_str(" · plist present but NOT LOADED");
            } else {
                line.push_str(" · loaded");
                if let Some(pid) = j.pid {
                    line.push_str(&format!(" (running, PID {pid})"));
                } else if let Some(exit) = j.last_exit {
                    line.push_str(&format!(" (last exit {exit})"));
                }
            }
            out.push_str(&line);
            out.push('\n');
        }
    }

    if let Some(log) = &r.log {
        out.push_str(&format!(
            "\nLog noise ({}): {} WARN · {} ERROR\n",
            log.file, log.warn_count, log.error_count
        ));
        if let Some(err) = &log.last_error {
            let shown: String = err.chars().take(160).collect();
            out.push_str(&format!("  last ERROR: {shown}\n"));
        }
    }

    if let Some(b) = &r.build {
        let fmt_local = |t: &DateTime<Utc>| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        };
        out.push_str(&format!("\nBuild: v{}\n", b.version));
        if let (Some(bin), Some(modified)) = (&b.binary, &b.binary_modified) {
            out.push_str(&format!("  binary   {bin} (modified {})\n", fmt_local(modified)));
        }
        if let Some(exe) = &b.daemon_exe
            && b.daemon_exe != b.binary
        {
            let modified = b
                .daemon_exe_modified
                .as_ref()
                .map(|m| format!(" (modified {})", fmt_local(m)))
                .unwrap_or_default();
            out.push_str(&format!("  daemon binary {exe}{modified}\n"));
        }
        if b.daemon_is_i_dream == Some(false) {
            out.push_str(
                "  ⚠ the PID in the pid file is not an i-dream process (stale or reused PID) — freshness verdicts withheld\n",
            );
        }
        if let Some(started) = &b.daemon_started {
            let verdict = match b.daemon_stale {
                Some(true) => " — STALE: its binary was replaced after it started",
                Some(false) => " — running its current binary",
                None => "",
            };
            out.push_str(&format!("  daemon   started {}{verdict}\n", fmt_local(started)));
        }
        if let Some(src) = &b.newest_source_change {
            let verdict = match b.binary_behind_source {
                Some(true) => " — BEHIND: source is newer than the binary",
                Some(false) => " — binary is up to date with source",
                None => "",
            };
            out.push_str(&format!("  source   newest change {}{verdict}\n", fmt_local(src)));
        }
        if b.daemon_stale == Some(true) {
            out.push_str("  ⚠ restart to load new code: i-dream service start\n");
        }
        if b.binary_behind_source == Some(true) {
            out.push_str("  ⚠ rebuild + reinstall: scripts/install.sh\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(name: &'static str, status: LaneStatus, reason: &str) -> LaneHealth {
        LaneHealth {
            lane: name,
            status,
            reason: reason.to_string(),
            consumer: "test",
        }
    }

    fn section(lanes: Vec<LaneHealth>) -> LanesSection {
        let (mut g, mut y, mut r) = (0, 0, 0);
        for l in &lanes {
            match l.status {
                LaneStatus::Green => g += 1,
                LaneStatus::Yellow => y += 1,
                LaneStatus::Red => r += 1,
            }
        }
        LanesSection {
            green: g,
            yellow: y,
            red: r,
            lanes,
        }
    }

    #[test]
    fn summary_line_all_green_shows_only_green() {
        let s = section(vec![
            lane("a", LaneStatus::Green, "fresh"),
            lane("b", LaneStatus::Green, "fresh"),
        ]);
        assert_eq!(lanes_summary_line(&s), "Lanes: 2 green");
    }

    #[test]
    fn summary_line_names_red_and_yellow_with_reasons_red_first() {
        let s = section(vec![
            lane("a", LaneStatus::Green, "fresh"),
            lane("warm", LaneStatus::Yellow, "aging 30h (cadence 24h)"),
            lane("dead", LaneStatus::Red, "store absent (x.jsonl)"),
        ]);
        let line = lanes_summary_line(&s);
        assert_eq!(
            line,
            "Lanes: 1 green · 1 yellow · 1 red \
             (dead: store absent (x.jsonl) · warm: aging 30h (cadence 24h))"
        );
    }

    #[test]
    fn scan_log_counts_levels_and_keeps_last_error() {
        let content = "\
2026-07-17T00:34:42.770558Z  WARN Hook event handler failed: Broken pipe (os error 32)
2026-07-17T00:35:00.000000Z  INFO cycle 1311 complete
2026-07-17T00:36:00.000000Z ERROR first failure
2026-07-17T00:37:00.000000Z  WARN another warning
2026-07-17T00:38:00.000000Z ERROR second failure";
        let (warns, errors, last) = scan_log(content);
        assert_eq!(warns, 2);
        assert_eq!(errors, 2);
        assert!(last.unwrap().contains("second failure"));
    }

    #[test]
    fn scan_log_empty_input_is_all_zero() {
        assert_eq!(scan_log(""), (0, 0, None));
    }

    #[test]
    fn parse_lstart_handles_padded_and_single_digit_days() {
        let t = parse_lstart("Thu Jul 17 11:50:23 2026\n").unwrap();
        assert_eq!(t.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-07-17 11:50:23");
        // BSD ps double-spaces single-digit days.
        let t = parse_lstart("Fri Jul  3 09:05:01 2026").unwrap();
        assert_eq!(t.format("%Y-%m-%d").to_string(), "2026-07-03");
    }

    #[test]
    fn parse_lstart_rejects_garbage() {
        assert!(parse_lstart("").is_none());
        assert!(parse_lstart("not a date at all").is_none());
    }

    #[test]
    fn parse_launchctl_list_extracts_pid_exit_and_program() {
        let out = r#"{
	"LimitLoadToSessionType" = "Aqua";
	"Label" = "com.alcatraz.i-dream-daily";
	"LastExitStatus" = 0;
	"PID" = 4242;
	"Program" = "/usr/local/bin/i-dream";
};"#;
        assert_eq!(
            parse_launchctl_list(out),
            (Some(4242), Some(0), Some("/usr/local/bin/i-dream".to_string()))
        );
    }

    #[test]
    fn parse_launchctl_list_tolerates_missing_pid() {
        let out = "\t\"LastExitStatus\" = 78;\n";
        assert_eq!(parse_launchctl_list(out), (None, Some(78), None));
    }

    #[test]
    fn parse_launchctl_list_drops_non_positive_pids() {
        // Real launchctl never emits these; a hostile or corrupt line must
        // read as "not running", not as PID -5.
        assert_eq!(parse_launchctl_list("\t\"PID\" = -5;\n").0, None);
        assert_eq!(parse_launchctl_list("\t\"PID\" = 0;\n").0, None);
    }

    // ── freshness_verdicts — pins each verdict's SOURCE ──────────────
    // The Batch A gate proved a mutation swapping daemon_exe_modified for
    // binary_modified stayed green across 492 tests. These pin it red.

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn daemon_stale_compares_the_daemons_exe_not_this_binary() {
        // Daemon started AFTER its own exe was built (fresh), but THIS
        // process's binary is newer than the daemon's start (a dev build).
        // Using binary_modified here would wrongly report stale.
        let verdict = freshness_verdicts(
            Some(t(100)), // daemon started
            true,
            Some(t(50)),  // daemon exe built before start → fresh
            Some(t(200)), // status binary newer than start → must be ignored
            None,
        );
        assert_eq!(verdict.0, Some(false));

        // And the true-stale direction still reads from the daemon's exe.
        let verdict = freshness_verdicts(
            Some(t(100)),
            true,
            Some(t(150)), // exe replaced after start → stale
            Some(t(0)),   // status binary older — must be ignored
            None,
        );
        assert_eq!(verdict.0, Some(true));
    }

    #[test]
    fn same_second_deploy_is_not_stale() {
        // install.sh replaces the binary and respawns the daemon inside one
        // second; ps lstart truncates to the second, so the start can read
        // as (sub-second) earlier than the mtime. That must not be STALE —
        // caught live on the first deploy of this feature (2026-07-17).
        let started = DateTime::<Utc>::from_timestamp(1_700_000_100, 0).unwrap();
        let built_same_second = DateTime::<Utc>::from_timestamp(1_700_000_100, 272_000_000).unwrap();
        let verdict = freshness_verdicts(
            Some(started),
            true,
            Some(built_same_second),
            None,
            None,
        );
        assert_eq!(verdict.0, Some(false));

        // A binary a full second+ newer is genuinely a replacement.
        let built_later = DateTime::<Utc>::from_timestamp(1_700_000_102, 0).unwrap();
        let verdict = freshness_verdicts(Some(started), true, Some(built_later), None, None);
        assert_eq!(verdict.0, Some(true));
    }

    #[test]
    fn non_idream_pid_gets_no_freshness_verdicts() {
        // A recycled PID pointing at some other process: judging its
        // binary's mtime would be confidently wrong — verdicts withheld.
        let verdict = freshness_verdicts(
            Some(t(100)),
            false,
            Some(t(150)),
            Some(t(200)),
            Some(t(300)),
        );
        assert_eq!(verdict.0, None);
        // behind-source falls back to this process's binary (t=200) vs
        // source (t=300) — still answerable without trusting the stranger.
        assert_eq!(verdict.1, Some(true));
    }

    #[test]
    fn behind_source_uses_the_daemons_exe_when_daemon_runs() {
        // Source newer than the daemon's deployed exe but older than this
        // (dev) binary: deployed is what matters → behind.
        let verdict = freshness_verdicts(
            Some(t(100)),
            true,
            Some(t(50)),  // deployed exe
            Some(t(400)), // dev binary — must be ignored
            Some(t(300)), // source change
        );
        assert_eq!(verdict.1, Some(true));
    }

    #[test]
    fn json_report_carries_the_load_bearing_keys() {
        let report = StatusReport {
            daemon: DaemonSection {
                status: "stopped".into(),
                pid: None,
                pid_file: None,
            },
            state: None,
            state_error: None,
            lanes: section(vec![lane("a", LaneStatus::Red, "store absent (x)")]),
            queue: QueueSection {
                depth: 3,
                oldest: Some("6d".into()),
            },
            modules: vec![ModuleInit {
                name: "dreams",
                initialized: true,
            }],
            jobs: Some(vec![]),
            log: None,
            build: None,
        };
        let v = serde_json::to_value(&report).unwrap();
        assert_eq!(v["daemon"]["status"], "stopped");
        assert_eq!(v["lanes"]["red"], 1);
        assert_eq!(v["lanes"]["lanes"][0]["lane"], "a");
        assert_eq!(v["queue"]["depth"], 3);
        assert_eq!(v["modules"][0]["name"], "dreams");
        // Skipped sections must be absent, not null — consumers key on presence.
        assert!(v.get("log").is_none());
        assert!(v.get("build").is_none());
    }

    #[test]
    fn render_text_base_contains_lanes_line_and_modules() {
        let report = StatusReport {
            daemon: DaemonSection {
                status: "stopped".into(),
                pid: None,
                pid_file: None,
            },
            state: None,
            state_error: None,
            lanes: section(vec![lane("a", LaneStatus::Green, "fresh")]),
            queue: QueueSection {
                depth: 0,
                oldest: None,
            },
            modules: vec![ModuleInit {
                name: "dreams",
                initialized: false,
            }],
            jobs: None,
            log: None,
            build: None,
        };
        let text = render_text(&report, false);
        assert!(text.contains("Daemon: stopped"));
        assert!(text.contains("Lanes: 1 green"));
        assert!(text.contains("dreams: not initialized"));
        // depth 0 → no queue line
        assert!(!text.contains("Queue:"));
    }

    #[test]
    fn render_text_shows_queue_line_when_backlog_exists() {
        let report = StatusReport {
            daemon: DaemonSection {
                status: "stopped".into(),
                pid: None,
                pid_file: None,
            },
            state: None,
            state_error: None,
            lanes: section(vec![]),
            queue: QueueSection {
                depth: 51,
                oldest: Some("6d".into()),
            },
            modules: vec![],
            jobs: None,
            log: None,
            build: None,
        };
        let text = render_text(&report, false);
        assert!(text.contains("Queue: 51 pending · oldest 6d"));
    }
}
