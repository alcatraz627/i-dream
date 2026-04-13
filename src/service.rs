//! Background service management — launchd LaunchAgent on macOS.
//!
//! The i-dream daemon is designed to run under a supervisor: it does
//! NOT fork itself into the background, it does NOT write its own log
//! rotation (that's what `logging::init` is for), and it does NOT try
//! to survive its own panics. All of that is delegated to launchd,
//! which knows how to do it right.
//!
//! ## Lifecycle
//!
//! ```text
//!   i-dream service install     launchctl bootstrap gui/$UID <plist>
//!   i-dream service uninstall   launchctl bootout   gui/$UID <plist> + rm plist
//!   i-dream service start       launchctl kickstart -k gui/$UID/<label>
//!   i-dream service stop        launchctl stop      gui/$UID/<label>
//!   i-dream service status      launchctl print     gui/$UID/<label>
//!   i-dream service logs        tail logs/i-dream.log.<YYYY-MM-DD>
//! ```
//!
//! ## Why launchd and not nohup/& shell tricks
//!
//! Three reasons:
//!   1. **Restart on crash.** `KeepAlive={SuccessfulExit=false, Crashed=true}`
//!      tells launchd to restart the daemon if it crashes but leave it
//!      stopped if we exited cleanly (i.e. the operator ran `service stop`).
//!      This is exactly the semantic we want — no extra state machine.
//!   2. **Start on login.** `RunAtLoad=true` means the daemon boots
//!      automatically when the user logs in. No cron hack, no `.zshrc`
//!      pollution.
//!   3. **Crash-loop throttling.** `ThrottleInterval=10` keeps launchd
//!      from spinning the daemon in a tight restart loop if something
//!      is badly wrong (e.g. missing API key). The daemon gets 10s
//!      of breathing room between restarts.
//!
//! ## Secrets
//!
//! The plist does NOT embed `ANTHROPIC_API_KEY` directly. Instead, the
//! daemon's `main()` loads it via `dotenvy` from
//! `~/.claude/subconscious/.env` — which the operator creates once by
//! hand and keeps out of version control. Plists get backed up to
//! iCloud in some setups, and embedding the API key there is a foot-gun.

use crate::cli::ServiceAction;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

/// Reverse-DNS label for the LaunchAgent. This is also the identifier
/// that `launchctl print`, `launchctl kickstart`, etc. use to refer to
/// the service. Changing this value orphans any already-installed
/// plist and requires a manual `launchctl bootout` + `service install`.
const AGENT_LABEL: &str = "dev.i-dream.daemon";

/// Dispatch from `main.rs`.
pub fn manage(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Install => install(),
        ServiceAction::Uninstall => uninstall(),
        ServiceAction::Start => start(),
        ServiceAction::Stop => stop(),
        ServiceAction::Status => status(),
        ServiceAction::Logs { lines } => logs(lines),
    }
}

// ─── install / uninstall ─────────────────────────────────────────

fn install() -> Result<()> {
    let paths = Paths::resolve()?;
    fs::create_dir_all(&paths.launch_agents_dir).with_context(|| {
        format!(
            "Failed to create LaunchAgents dir at {}",
            paths.launch_agents_dir.display()
        )
    })?;
    fs::create_dir_all(&paths.data_dir)?;
    fs::create_dir_all(&paths.logs_dir)?;

    // Render and write the plist.
    let plist_body = render_plist(&paths);
    fs::write(&paths.plist_path, &plist_body).with_context(|| {
        format!("Failed to write plist to {}", paths.plist_path.display())
    })?;
    info!("Wrote plist to {}", paths.plist_path.display());

    // `launchctl bootstrap` loads the plist into the gui/$UID domain.
    // This is the modern replacement for the deprecated `launchctl load`.
    let domain = gui_domain()?;
    let output = Command::new("launchctl")
        .args(["bootstrap", &domain, &paths.plist_path.display().to_string()])
        .output()
        .context("Failed to invoke launchctl bootstrap")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // If the service is already loaded, `bootstrap` returns
        // exit code 37 ("service is already loaded"). Treat that as
        // a non-fatal reinstall hint.
        if stderr.contains("already loaded") || stderr.contains("service already") {
            println!(
                "Service is already loaded. Run `i-dream service uninstall` first \
                 if you want to reinstall."
            );
            return Ok(());
        }
        anyhow::bail!(
            "launchctl bootstrap failed (exit {}): {stderr}",
            output.status.code().unwrap_or(-1)
        );
    }

    println!("Service installed and loaded: {AGENT_LABEL}");
    println!("Plist:  {}", paths.plist_path.display());
    println!("Logs:   {}", paths.logs_dir.display());
    println!("Start:  i-dream service start");
    Ok(())
}

fn uninstall() -> Result<()> {
    let paths = Paths::resolve()?;
    let domain = gui_domain()?;

    if paths.plist_path.exists() {
        let output = Command::new("launchctl")
            .args(["bootout", &format!("{domain}/{AGENT_LABEL}")])
            .output()
            .context("Failed to invoke launchctl bootout")?;

        if !output.status.success() {
            // `bootout` on a non-loaded service returns a non-zero exit
            // code (113 "could not find specified service"). That's
            // fine — we're about to delete the plist file anyway, and
            // the end state is the same.
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("Could not find") && !stderr.contains("not find") {
                eprintln!(
                    "launchctl bootout warning: {stderr}"
                );
            }
        }

        fs::remove_file(&paths.plist_path).with_context(|| {
            format!("Failed to delete plist at {}", paths.plist_path.display())
        })?;
        println!("Service uninstalled: {AGENT_LABEL}");
    } else {
        println!("No plist installed at {}", paths.plist_path.display());
    }
    Ok(())
}

// ─── start / stop / status / logs ────────────────────────────────

fn start() -> Result<()> {
    let paths = Paths::resolve()?;
    if !paths.plist_path.exists() {
        anyhow::bail!(
            "Service not installed. Run `i-dream service install` first."
        );
    }
    let domain = gui_domain()?;
    let output = Command::new("launchctl")
        .args(["kickstart", "-k", &format!("{domain}/{AGENT_LABEL}")])
        .output()
        .context("Failed to invoke launchctl kickstart")?;

    if !output.status.success() {
        anyhow::bail!(
            "launchctl kickstart failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    println!("Service kicked: {AGENT_LABEL}");
    Ok(())
}

fn stop() -> Result<()> {
    let domain = gui_domain()?;
    let output = Command::new("launchctl")
        .args(["stop", AGENT_LABEL])
        .output()
        .context("Failed to invoke launchctl stop")?;
    let _ = domain; // (informational; `stop` uses the label directly)

    if !output.status.success() {
        anyhow::bail!(
            "launchctl stop failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    println!("Service stopped: {AGENT_LABEL}");
    Ok(())
}

fn status() -> Result<()> {
    let paths = Paths::resolve()?;
    let domain = gui_domain()?;

    println!("Service label: {AGENT_LABEL}");
    println!(
        "Plist installed: {}",
        if paths.plist_path.exists() { "yes" } else { "no" }
    );

    let output = Command::new("launchctl")
        .args(["print", &format!("{domain}/{AGENT_LABEL}")])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            // `launchctl print` is verbose — pull out the two most
            // operationally interesting lines: "state" and "last exit code".
            let out = String::from_utf8_lossy(&o.stdout);
            for line in out.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("state =")
                    || trimmed.starts_with("last exit code")
                    || trimmed.starts_with("pid =")
                {
                    println!("  {trimmed}");
                }
            }
        }
        Ok(o) => {
            println!(
                "  launchctl print failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Err(e) => println!("  launchctl print errored: {e}"),
    }

    // Cross-check against the PID file, which is managed by the daemon
    // itself — this catches the case where launchd thinks the service
    // is "running" but the daemon hasn't actually acquired its PID lock
    // yet (very short window during startup).
    let pid_path = paths.data_dir.join("daemon.pid");
    if pid_path.exists() {
        let content = fs::read_to_string(&pid_path).unwrap_or_default();
        println!("  PID file: {} ({})", pid_path.display(), content.trim());
    } else {
        println!("  PID file: absent");
    }
    Ok(())
}

fn logs(lines: usize) -> Result<()> {
    let paths = Paths::resolve()?;
    let latest = latest_log_file(&paths.logs_dir)?;
    let Some(latest) = latest else {
        println!("No log files in {}", paths.logs_dir.display());
        return Ok(());
    };

    let content = fs::read_to_string(&latest)
        .with_context(|| format!("Failed to read {}", latest.display()))?;
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(lines);
    println!("─── {} (last {} lines) ───", latest.display(), lines);
    for line in &all[start..] {
        println!("{line}");
    }
    Ok(())
}

// ─── helpers ─────────────────────────────────────────────────────

/// Resolved filesystem paths for a given install. Computed once and
/// threaded through the handlers so every command sees a consistent
/// view of where things live.
struct Paths {
    launch_agents_dir: PathBuf,
    plist_path: PathBuf,
    data_dir: PathBuf,
    logs_dir: PathBuf,
    binary_path: PathBuf,
}

impl Paths {
    fn resolve() -> Result<Self> {
        let home = dirs::home_dir().context("Could not resolve home directory")?;
        let launch_agents_dir = home.join("Library/LaunchAgents");
        let plist_path = launch_agents_dir.join(format!("{AGENT_LABEL}.plist"));
        let data_dir = home.join(".claude/subconscious");
        let logs_dir = data_dir.join("logs");

        // `current_exe()` returns the path of the currently running
        // binary. This is exactly what we want — the plist will reference
        // whatever copy of i-dream the operator ran `service install` with,
        // so `cargo install` → `~/.cargo/bin/i-dream` vs a local build
        // target both work without manual PATH surgery.
        let binary_path = std::env::current_exe()
            .context("Could not resolve current executable path")?
            .canonicalize()
            .unwrap_or_else(|_| std::env::current_exe().unwrap());

        Ok(Self {
            launch_agents_dir,
            plist_path,
            data_dir,
            logs_dir,
            binary_path,
        })
    }
}

/// Build the `gui/$UID` domain specifier that launchctl bootstrap needs.
fn gui_domain() -> Result<String> {
    // `users::get_current_uid()` would add a dep; use libc directly.
    let uid = unsafe { libc::getuid() };
    Ok(format!("gui/{uid}"))
}

/// Find the most recently modified rolling log file in `logs_dir`.
/// Filters by `i-dream.log` prefix so we don't accidentally tail
/// `events.jsonl` or some other companion file.
fn latest_log_file(logs_dir: &Path) -> Result<Option<PathBuf>> {
    if !logs_dir.exists() {
        return Ok(None);
    }
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(logs_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        if !name_str.starts_with("i-dream.log") {
            continue;
        }
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if best.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            best = Some((modified, entry.path()));
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// Render the LaunchAgent plist body for these `Paths`.
///
/// The plist uses the `KeepAlive` *dict* form (not the boolean form)
/// because boolean `KeepAlive=true` would restart us even after a clean
/// `service stop`, which is not what the operator wants. The dict form
/// lets us say "restart on crash, stay dead on clean exit".
fn render_plist(paths: &Paths) -> String {
    let stdout_path = paths.logs_dir.join("launchd.stdout.log");
    let stderr_path = paths.logs_dir.join("launchd.stderr.log");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>start</string>
        <string>--daemonize</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
        <key>Crashed</key>
        <true/>
    </dict>

    <key>ThrottleInterval</key>
    <integer>10</integer>

    <key>Nice</key>
    <integer>10</integer>

    <key>WorkingDirectory</key>
    <string>{data_dir}</string>

    <key>StandardOutPath</key>
    <string>{stdout_log}</string>

    <key>StandardErrorPath</key>
    <string>{stderr_log}</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>{home}</string>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin</string>
    </dict>

    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#,
        label = AGENT_LABEL,
        binary = paths.binary_path.display(),
        data_dir = paths.data_dir.display(),
        stdout_log = stdout_path.display(),
        stderr_log = stderr_path.display(),
        home = dirs::home_dir().unwrap_or_default().display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The plist template is the most error-prone piece of this module
    // — a bad plist either refuses to load (`launchctl bootstrap`
    // returns a cryptic error) or loads but runs something wrong.
    // These tests pin the invariants that matter most operationally.

    fn paths_fixture() -> Paths {
        Paths {
            launch_agents_dir: PathBuf::from("/Users/test/Library/LaunchAgents"),
            plist_path: PathBuf::from(
                "/Users/test/Library/LaunchAgents/dev.i-dream.daemon.plist",
            ),
            data_dir: PathBuf::from("/Users/test/.claude/subconscious"),
            logs_dir: PathBuf::from("/Users/test/.claude/subconscious/logs"),
            binary_path: PathBuf::from("/Users/test/.cargo/bin/i-dream"),
        }
    }

    #[test]
    fn plist_contains_absolute_binary_path() {
        let plist = render_plist(&paths_fixture());
        assert!(
            plist.contains("<string>/Users/test/.cargo/bin/i-dream</string>"),
            "plist must reference the absolute binary path; got:\n{plist}"
        );
    }

    #[test]
    fn plist_uses_keepalive_dict_form_not_boolean() {
        // Boolean KeepAlive would restart us even on clean exit,
        // which defeats `i-dream service stop`. Dict form is required.
        let plist = render_plist(&paths_fixture());
        assert!(
            plist.contains("<key>KeepAlive</key>\n    <dict>"),
            "KeepAlive must be a <dict>, not <true/>"
        );
        assert!(
            plist.contains("<key>SuccessfulExit</key>\n        <false/>"),
            "SuccessfulExit must be false (don't restart on clean exit)"
        );
        assert!(
            plist.contains("<key>Crashed</key>\n        <true/>"),
            "Crashed must be true (restart on crash)"
        );
    }

    #[test]
    fn plist_declares_throttle_interval() {
        // Without ThrottleInterval a crashing daemon will hammer
        // launchd in a tight loop. 10s is long enough that an operator
        // notices, short enough that a recovering daemon gets back
        // online promptly.
        let plist = render_plist(&paths_fixture());
        assert!(
            plist.contains("<key>ThrottleInterval</key>\n    <integer>10</integer>"),
            "ThrottleInterval must be set to 10"
        );
    }

    #[test]
    fn plist_redirects_stdout_and_stderr_into_logs_dir() {
        let plist = render_plist(&paths_fixture());
        assert!(
            plist.contains("<string>/Users/test/.claude/subconscious/logs/launchd.stdout.log</string>"),
            "stdout must land in logs/launchd.stdout.log"
        );
        assert!(
            plist.contains("<string>/Users/test/.claude/subconscious/logs/launchd.stderr.log</string>"),
            "stderr must land in logs/launchd.stderr.log"
        );
    }

    #[test]
    fn plist_does_not_embed_api_key() {
        // The API key lives in ~/.claude/subconscious/.env and is
        // loaded by `main()` via dotenvy. Embedding it in the plist
        // would leak it into iCloud plist backups.
        let plist = render_plist(&paths_fixture());
        assert!(
            !plist.contains("ANTHROPIC_API_KEY"),
            "plist must not reference ANTHROPIC_API_KEY — use .env instead"
        );
    }

    #[test]
    fn plist_label_matches_agent_label_constant() {
        let plist = render_plist(&paths_fixture());
        assert!(
            plist.contains(&format!("<string>{AGENT_LABEL}</string>")),
            "plist Label must equal AGENT_LABEL"
        );
    }

    #[test]
    fn plist_program_arguments_has_daemonize_flag() {
        // Without --daemonize the daemon won't acquire the PID file,
        // which means `status` and `stop` can't find it.
        let plist = render_plist(&paths_fixture());
        assert!(
            plist.contains("<string>--daemonize</string>"),
            "ProgramArguments must include --daemonize"
        );
    }

    #[test]
    fn latest_log_file_picks_most_recently_modified() {
        let dir = tempfile::tempdir().unwrap();
        let older = dir.path().join("i-dream.log.2026-04-10");
        let newer = dir.path().join("i-dream.log.2026-04-12");
        fs::write(&older, "old").unwrap();
        fs::write(&newer, "new").unwrap();

        // Backdate the older file to ensure deterministic ordering.
        let file = fs::File::options().write(true).open(&older).unwrap();
        file.set_modified(
            std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
        )
        .unwrap();

        let latest = latest_log_file(dir.path()).unwrap();
        assert_eq!(latest, Some(newer));
    }

    #[test]
    fn latest_log_file_ignores_non_log_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("events.jsonl"), "stuff").unwrap();
        fs::write(dir.path().join("unrelated.txt"), "stuff").unwrap();
        let latest = latest_log_file(dir.path()).unwrap();
        assert_eq!(latest, None);
    }

    #[test]
    fn latest_log_file_on_missing_dir_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(latest_log_file(&missing).unwrap(), None);
    }
}
