//! Menubar widget management.
//!
//! Wraps `tools/menubar/build.sh` and direct process management for
//! the i-dream-bar Swift menubar widget. The binary lives next to the
//! script at `tools/menubar/i-dream-bar` relative to the cargo workspace
//! root (discovered via the `CARGO_MANIFEST_DIR` env-var baked in at
//! compile time, or by walking up from the current executable path).
//!
//! ## Commands
//!
//! ```text
//!   i-dream widget start     launch widget (no recompile)
//!   i-dream widget stop      kill all running widget instances
//!   i-dream widget restart   stop + start
//!   i-dream widget build     recompile from source + relaunch
//!   i-dream widget status    show PID, LaunchAgent state, build freshness
//!   i-dream widget logs      tail /tmp/i-dream-bar.log
//!   i-dream widget install   register as LaunchAgent (auto-start on login)
//!   i-dream widget uninstall remove LaunchAgent
//! ```

use crate::cli::WidgetAction;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

const BINARY_NAME: &str = "i-dream-bar";
const LAUNCHD_LABEL: &str = "dev.i-dream.menubar";
const DEBUG_LOG: &str = "/tmp/i-dream-bar.log";

pub fn manage(action: WidgetAction) -> Result<()> {
    match action {
        WidgetAction::Start   => start(),
        WidgetAction::Stop    => stop(),
        WidgetAction::Restart => restart(),
        WidgetAction::Build   => build(),
        WidgetAction::Status  => status(),
        WidgetAction::Logs { lines } => logs(lines),
        WidgetAction::Install   => run_build_sh(&["--install"]),
        WidgetAction::Uninstall => run_build_sh(&["--uninstall"]),
    }
}

// ─── actions ─────────────────────────────────────────────────────────────────

fn start() -> Result<()> {
    if is_running() {
        println!("Widget is already running (PID {}).", current_pid().unwrap_or(0));
        return Ok(());
    }
    let bin = widget_binary()?;
    // Launch detached: stdout/stderr → debug log, no controlling terminal.
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(DEBUG_LOG)
        .with_context(|| format!("Cannot open debug log at {DEBUG_LOG}"))?;
    let log_copy = log_file.try_clone()?;
    Command::new(&bin)
        .stdout(log_file)
        .stderr(log_copy)
        .spawn()
        .with_context(|| format!("Failed to launch {}", bin.display()))?;

    // Brief wait so pgrep has time to register the process.
    std::thread::sleep(std::time::Duration::from_millis(600));
    if let Some(pid) = current_pid() {
        println!("Widget started (PID {pid}).");
        println!("Logs: tail -f {DEBUG_LOG}");
    } else {
        println!("Widget launched but did not appear in process list — check {DEBUG_LOG}");
    }
    Ok(())
}

fn stop() -> Result<()> {
    if !is_running() {
        println!("Widget is not running.");
        return Ok(());
    }
    let output = Command::new("pkill")
        .args(["-x", BINARY_NAME])
        .output()
        .context("Failed to invoke pkill")?;
    if output.status.success() || output.status.code() == Some(1) {
        // pkill exit 1 = no process matched (already gone).
        println!("Widget stopped.");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("pkill failed: {stderr}");
    }
    Ok(())
}

fn restart() -> Result<()> {
    stop()?;
    // Give the OS a moment to clean up before relaunching.
    std::thread::sleep(std::time::Duration::from_millis(400));
    start()
}

fn build() -> Result<()> {
    run_build_sh(&[])
}

fn status() -> Result<()> {
    // ── Process ──────────────────────────────────────────────────────────────
    println!("Process:");
    if let Some(pid) = current_pid() {
        println!("  Running  PID {pid}");
    } else {
        println!("  Not running");
    }

    // ── LaunchAgent ──────────────────────────────────────────────────────────
    println!("\nLaunchAgent ({LAUNCHD_LABEL}):");
    let la_out = Command::new("launchctl")
        .args(["list", LAUNCHD_LABEL])
        .output()
        .context("Failed to invoke launchctl")?;
    if la_out.status.success() {
        let stdout = String::from_utf8_lossy(&la_out.stdout);
        // `launchctl list <label>` prints a plist-like dict; pull out PID + LastExitStatus.
        for line in stdout.lines() {
            let t = line.trim();
            if t.contains("\"PID\"") || t.contains("\"LastExitStatus\"") || t.contains("\"Label\"") {
                println!("  {t}");
            }
        }
    } else {
        println!("  Not registered (run `i-dream widget install` to enable auto-start)");
    }

    // ── Build freshness ──────────────────────────────────────────────────────
    println!("\nBuild:");
    if let Ok(build_info_path) = build_info_path() {
        if build_info_path.exists() {
            let info = std::fs::read_to_string(&build_info_path)
                .unwrap_or_default();
            let get = |key: &str| -> String {
                info.lines()
                    .find(|l| l.starts_with(key))
                    .and_then(|l| l.splitn(2, '=').nth(1))
                    .unwrap_or("?")
                    .to_string()
            };
            let commit = get("commit");
            let built_at = get("built_at");
            println!("  Built at: {built_at}  (commit: {commit})");

            // Compare source hash to detect staleness.
            if let Ok(source) = source_path() {
                let md5_out = Command::new("md5")
                    .arg(&source)
                    .output()
                    .ok();
                if let Some(out) = md5_out {
                    let current_hash: String = String::from_utf8_lossy(&out.stdout)
                        .split_whitespace()
                        .last()
                        .map(|s| s.chars().take(8).collect())
                        .unwrap_or_default();
                    let built_hash = get("src_hash");
                    if current_hash == built_hash {
                        println!("  Source:   ✓ Binary matches source (hash: {current_hash})");
                    } else {
                        println!("  Source:   ⚠ SOURCE HAS CHANGED — binary is stale!");
                        println!("            source now:  {current_hash}");
                        println!("            binary from: {built_hash}");
                        println!("            → run: i-dream widget build");
                    }
                }
            }
        } else {
            println!("  (no .build-info — binary predates hash tracking)");
        }
    }

    Ok(())
}

fn logs(lines: usize) -> Result<()> {
    let status = Command::new("tail")
        .args(["-n", &lines.to_string(), DEBUG_LOG])
        .status()
        .context("Failed to invoke tail")?;
    if !status.success() {
        bail!("tail exited non-zero — {DEBUG_LOG} may not exist yet");
    }
    Ok(())
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Run build.sh with the given extra args, inheriting stdout/stderr so the
/// compile output streams directly to the user's terminal.
fn run_build_sh(extra_args: &[&str]) -> Result<()> {
    let script = build_sh_path()?;
    let mut cmd = Command::new("bash");
    cmd.arg(&script);
    for a in extra_args { cmd.arg(a); }
    let status = cmd.status()
        .with_context(|| format!("Failed to run {}", script.display()))?;
    if !status.success() {
        bail!("build.sh exited with status {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

fn is_running() -> bool {
    Command::new("pgrep")
        .args(["-x", BINARY_NAME])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn current_pid() -> Option<u32> {
    let out = Command::new("pgrep")
        .args(["-x", BINARY_NAME])
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|s| s.trim().parse().ok())
}

/// Resolve the project root by walking up from the running executable until
/// we find a `tools/menubar/build.sh`. Falls back to `CARGO_MANIFEST_DIR`
/// (only available in `cargo run`).
fn project_root() -> Result<PathBuf> {
    // 1. Walk up from the executable.
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        for _ in 0..8 {
            if let Some(d) = dir {
                if d.join("tools/menubar/build.sh").exists() {
                    return Ok(d);
                }
                dir = d.parent().map(|p| p.to_path_buf());
            } else {
                break;
            }
        }
    }
    // 2. Compile-time fallback (cargo run / dev builds).
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        return Ok(PathBuf::from(manifest));
    }
    bail!("Could not locate project root (tools/menubar/build.sh not found up from executable)");
}

fn build_sh_path() -> Result<PathBuf> {
    Ok(project_root()?.join("tools/menubar/build.sh"))
}

fn widget_binary() -> Result<PathBuf> {
    Ok(project_root()?.join("tools/menubar").join(BINARY_NAME))
}

fn source_path() -> Result<PathBuf> {
    Ok(project_root()?.join("tools/menubar/i-dream-bar.swift"))
}

fn build_info_path() -> Result<PathBuf> {
    Ok(project_root()?.join("tools/menubar/.build-info"))
}
