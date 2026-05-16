//! launchd plist management for i-dream's scheduled jobs (B Stage 7 light).
//!
//! Today this ships only the daily digest plist (runs `i-dream digest` at
//! 03:00 local). The weekly audit plist depends on `i-dream audit` (B
//! Stage 5+6) and lands when that does. Catch-up logic + thread carry-over
//! also wait for those producers — see docs/16-consolidation-build.md §3.7-3.8.

use crate::cli::CronAction;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const DAILY_LABEL: &str = "com.alcatraz.i-dream-daily";

pub fn handle(action: CronAction) -> Result<()> {
    match action {
        CronAction::Install => install_daily(),
        CronAction::Uninstall => uninstall_daily(),
        CronAction::Status => status_daily(),
    }
}

fn install_daily() -> Result<()> {
    let binary = resolve_binary()?;
    let home = std::env::var("HOME").context("HOME unset")?;
    let plist_path = PathBuf::from(&home)
        .join("Library/LaunchAgents")
        .join(format!("{DAILY_LABEL}.plist"));
    let logs_dir = PathBuf::from(&home).join(".claude/i-dream/logs");
    fs::create_dir_all(&logs_dir)?;

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{DAILY_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
        <string>digest</string>
    </array>
    <key>StartCalendarInterval</key>
    <dict>
        <key>Hour</key><integer>3</integer>
        <key>Minute</key><integer>0</integer>
    </dict>
    <key>StandardOutPath</key>
    <string>{out}</string>
    <key>StandardErrorPath</key>
    <string>{err}</string>
    <key>RunAtLoad</key><false/>
</dict>
</plist>
"#,
        bin = binary.display(),
        out = logs_dir.join("daily.out.log").display(),
        err = logs_dir.join("daily.err.log").display(),
    );

    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plist_path, &plist)
        .with_context(|| format!("Cannot write {}", plist_path.display()))?;

    // Bootstrap (idempotent — bootout first, then bootstrap).
    let uid = unsafe { libc::getuid() };
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}/{DAILY_LABEL}")])
        .output();
    let bs = Command::new("launchctl")
        .args([
            "bootstrap",
            &format!("gui/{uid}"),
            plist_path.to_str().unwrap(),
        ])
        .output()
        .context("launchctl bootstrap failed")?;
    if !bs.status.success() {
        let err = String::from_utf8_lossy(&bs.stderr);
        bail!("launchctl bootstrap exited {}: {}", bs.status, err.trim());
    }

    println!("✓ Daily digest cron installed.");
    println!("   Label:    {DAILY_LABEL}");
    println!("   Schedule: 03:00 local daily");
    println!("   Program:  {} digest", binary.display());
    println!("   Plist:    {}", plist_path.display());
    println!("   Logs:     {}", logs_dir.display());
    println!();
    println!("   Status:    i-dream cron status");
    println!("   Uninstall: i-dream cron uninstall");
    Ok(())
}

fn uninstall_daily() -> Result<()> {
    let home = std::env::var("HOME").context("HOME unset")?;
    let plist_path = PathBuf::from(&home)
        .join("Library/LaunchAgents")
        .join(format!("{DAILY_LABEL}.plist"));

    let uid = unsafe { libc::getuid() };
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}/{DAILY_LABEL}")])
        .output();

    if plist_path.exists() {
        fs::remove_file(&plist_path)
            .with_context(|| format!("Cannot remove {}", plist_path.display()))?;
        println!("✓ Removed {}", plist_path.display());
    } else {
        println!("  (plist was already absent)");
    }
    Ok(())
}

fn status_daily() -> Result<()> {
    let home = std::env::var("HOME").context("HOME unset")?;
    let plist_path = PathBuf::from(&home)
        .join("Library/LaunchAgents")
        .join(format!("{DAILY_LABEL}.plist"));

    println!("Daily digest cron ({DAILY_LABEL}):");
    if !plist_path.exists() {
        println!("  Plist:     {} — NOT INSTALLED", plist_path.display());
        println!("  Run `i-dream cron install` to schedule the daily digest at 03:00.");
        return Ok(());
    }
    println!("  Plist:     {}", plist_path.display());

    let out = Command::new("launchctl")
        .args(["list", DAILY_LABEL])
        .output()
        .context("launchctl list failed")?;
    if out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            let t = line.trim();
            if t.contains("\"PID\"") || t.contains("\"LastExitStatus\"") || t.contains("\"Label\"")
            {
                println!("  {t}");
            }
        }
    } else {
        println!("  Registered: NO (plist exists but not loaded)");
        println!("  Run `i-dream cron install` to re-register.");
    }
    Ok(())
}

/// Resolve the `i-dream` binary to an absolute path for embedding in the
/// plist. Prefers the current `$0`, falls back to common install paths.
fn resolve_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        return Ok(exe);
    }
    let home = std::env::var("HOME").context("HOME unset")?;
    for candidate in [
        format!("{home}/.cargo/bin/i-dream"),
        format!("{home}/.local/bin/i-dream"),
        "/usr/local/bin/i-dream".to_string(),
        "/opt/homebrew/bin/i-dream".to_string(),
    ] {
        let p = PathBuf::from(&candidate);
        if p.exists() {
            return Ok(p);
        }
    }
    bail!("Cannot resolve i-dream binary path for plist Program field");
}
