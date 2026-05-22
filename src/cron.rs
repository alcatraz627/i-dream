//! launchd plist management for i-dream's scheduled jobs.
//!
//! A small job registry drives install/uninstall/status, so adding a job is a
//! one-line addition to `JOBS` rather than a new install/uninstall/status trio.
//! dream-pass runs just before the daily digest so the digest renders fresh
//! union views; the weekly audit prepares GCC proposals for the user to review.

use crate::cli::CronAction;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// When a job fires. `weekday` follows launchd's convention: 0 = Sunday.
enum Schedule {
    Daily { hour: u8, minute: u8 },
    Weekly { weekday: u8, hour: u8, minute: u8 },
}

struct CronJob {
    label: &'static str,
    args: &'static [&'static str],
    schedule: Schedule,
    desc: &'static str,
}

/// The scheduled jobs, in fire order. dream-pass first (populates the union
/// views), then the digest reads them; the weekly audit runs non-interactively
/// to stage proposals the user later reviews + applies.
const JOBS: &[CronJob] = &[
    CronJob {
        label: "com.alcatraz.i-dream-dreampass",
        args: &["dream-pass"],
        schedule: Schedule::Daily { hour: 2, minute: 45 },
        desc: "dream pass over fresh delta (02:45 daily; idle domains cost nothing)",
    },
    CronJob {
        label: "com.alcatraz.i-dream-daily",
        args: &["digest"],
        schedule: Schedule::Daily { hour: 3, minute: 0 },
        desc: "daily digest (03:00 daily)",
    },
    CronJob {
        label: "com.alcatraz.i-dream-audit",
        args: &["audit", "run", "--non-interactive"],
        schedule: Schedule::Weekly {
            weekday: 0,
            hour: 2,
            minute: 30,
        },
        desc: "weekly audit — stages proposals to review (Sun 02:30)",
    },
];

pub fn handle(action: CronAction) -> Result<()> {
    match action {
        CronAction::Install => install_all(),
        CronAction::Uninstall => uninstall_all(),
        CronAction::Status => status_all(),
    }
}

fn launch_agents_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents"))
}

fn plist_path(label: &str) -> Result<PathBuf> {
    Ok(launch_agents_dir()?.join(format!("{label}.plist")))
}

fn install_all() -> Result<()> {
    let binary = resolve_binary()?;
    let home = std::env::var("HOME").context("HOME unset")?;
    let logs_dir = PathBuf::from(&home).join(".claude/i-dream/logs");
    fs::create_dir_all(&logs_dir)?;
    let agents = launch_agents_dir()?;
    fs::create_dir_all(&agents)?;
    let uid = unsafe { libc::getuid() };

    for job in JOBS {
        let path = plist_path(job.label)?;
        let plist = render_plist(job, &binary, &logs_dir);
        fs::write(&path, &plist).with_context(|| format!("Cannot write {}", path.display()))?;

        // Idempotent: bootout (ignore failure if not loaded), then bootstrap.
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{}", job.label)])
            .output();
        let bs = Command::new("launchctl")
            .args(["bootstrap", &format!("gui/{uid}"), path.to_str().unwrap()])
            .output()
            .context("launchctl bootstrap failed")?;
        if !bs.status.success() {
            let err = String::from_utf8_lossy(&bs.stderr);
            bail!(
                "launchctl bootstrap for {} exited {}: {}",
                job.label,
                bs.status,
                err.trim()
            );
        }
        println!("✓ {} — {}", job.label, job.desc);
    }

    println!();
    println!("   Program:  {}", binary.display());
    println!("   Logs:     {}", logs_dir.display());
    println!("   Status:    i-dream cron status");
    println!("   Uninstall: i-dream cron uninstall");
    Ok(())
}

fn uninstall_all() -> Result<()> {
    let uid = unsafe { libc::getuid() };
    for job in JOBS {
        let path = plist_path(job.label)?;
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{}", job.label)])
            .output();
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Cannot remove {}", path.display()))?;
            println!("✓ Removed {}", path.display());
        } else {
            println!("  ({} already absent)", job.label);
        }
    }
    Ok(())
}

fn status_all() -> Result<()> {
    for job in JOBS {
        let path = plist_path(job.label)?;
        println!("{} — {}", job.label, job.desc);
        if !path.exists() {
            println!("  NOT INSTALLED (run `i-dream cron install`)");
            continue;
        }
        let out = Command::new("launchctl")
            .args(["list", job.label])
            .output()
            .context("launchctl list failed")?;
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let t = line.trim();
                if t.contains("\"PID\"") || t.contains("\"LastExitStatus\"") {
                    println!("  {t}");
                }
            }
        } else {
            println!("  plist present but not loaded — run `i-dream cron install`");
        }
    }
    Ok(())
}

/// Render a launchd plist for one job. `StartCalendarInterval` carries the
/// weekday only for weekly jobs (launchd treats an absent key as "every day").
fn render_plist(job: &CronJob, binary: &std::path::Path, logs_dir: &std::path::Path) -> String {
    let args_xml = std::iter::once(binary.display().to_string())
        .chain(job.args.iter().map(|a| a.to_string()))
        .map(|a| format!("        <string>{}</string>", xml_escape(&a)))
        .collect::<Vec<_>>()
        .join("\n");

    let cal = match job.schedule {
        Schedule::Daily { hour, minute } => format!(
            "        <key>Hour</key><integer>{hour}</integer>\n        <key>Minute</key><integer>{minute}</integer>"
        ),
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => format!(
            "        <key>Weekday</key><integer>{weekday}</integer>\n        <key>Hour</key><integer>{hour}</integer>\n        <key>Minute</key><integer>{minute}</integer>"
        ),
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{args_xml}
    </array>
    <key>StartCalendarInterval</key>
    <dict>
{cal}
    </dict>
    <key>StandardOutPath</key>
    <string>{out}</string>
    <key>StandardErrorPath</key>
    <string>{err}</string>
    <key>RunAtLoad</key><false/>
</dict>
</plist>
"#,
        label = job.label,
        out = xml_escape(&logs_dir.join(format!("{}.out.log", job.label)).display().to_string()),
        err = xml_escape(&logs_dir.join(format!("{}.err.log", job.label)).display().to_string()),
    )
}

/// Escape the XML metacharacters that would otherwise break the plist if a
/// path or argument contained them (e.g. an install path with `&`).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Resolve the `i-dream` binary to an absolute path for the plist Program
/// field. Prefers the running executable, falls back to common install paths.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn daily_plist_has_hour_minute_no_weekday() {
        let job = &JOBS[0]; // dream-pass, daily
        let p = render_plist(job, Path::new("/usr/local/bin/i-dream"), Path::new("/tmp/logs"));
        assert!(p.contains("<key>Hour</key><integer>2</integer>"));
        assert!(p.contains("<key>Minute</key><integer>45</integer>"));
        assert!(!p.contains("<key>Weekday</key>"));
        assert!(p.contains("<string>dream-pass</string>"));
    }

    #[test]
    fn weekly_plist_carries_weekday() {
        let job = JOBS.iter().find(|j| j.label.ends_with("audit")).unwrap();
        let p = render_plist(job, Path::new("/usr/local/bin/i-dream"), Path::new("/tmp/logs"));
        assert!(p.contains("<key>Weekday</key><integer>0</integer>"));
        assert!(p.contains("<string>audit</string>"));
        assert!(p.contains("<string>--non-interactive</string>"));
    }

    #[test]
    fn job_labels_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for job in JOBS {
            assert!(seen.insert(job.label), "duplicate job label: {}", job.label);
        }
    }
}
