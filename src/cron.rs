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
pub(crate) enum Schedule {
    Daily { hour: u8, minute: u8 },
    Weekly { weekday: u8, hour: u8, minute: u8 },
}

impl Schedule {
    /// Compact human phrase for status surfaces: "daily 02:45" / "Sun 02:30".
    pub(crate) fn human(&self) -> String {
        match self {
            Schedule::Daily { hour, minute } => format!("daily {hour:02}:{minute:02}"),
            Schedule::Weekly {
                weekday,
                hour,
                minute,
            } => {
                let day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
                    .get(*weekday as usize)
                    .copied()
                    .unwrap_or("?");
                format!("{day} {hour:02}:{minute:02}")
            }
        }
    }

    /// The next wall-clock instant this schedule fires strictly after `now`,
    /// mirroring launchd's local-time StartCalendarInterval semantics.
    /// Generic over the timezone so DST behavior is testable with fixed
    /// zones; production callers pass `Local::now()`.
    pub(crate) fn next_fire_after<Tz: chrono::TimeZone>(
        &self,
        now: chrono::DateTime<Tz>,
    ) -> Option<chrono::DateTime<Tz>> {
        use chrono::{Datelike, Duration as CDuration};
        let (hour, minute, want_weekday) = match self {
            Schedule::Daily { hour, minute } => (*hour, *minute, None),
            Schedule::Weekly {
                weekday,
                hour,
                minute,
            } => (*hour, *minute, Some(*weekday as u32)),
        };
        // Walk day by day instead of doing modular weekday arithmetic —
        // slower by nanoseconds, immune to off-by-one bugs. The window is
        // TWO weeks: a weekly schedule has one matching day per week, and
        // if that day's local fire time doesn't exist (DST spring-forward
        // gap — e.g. Sun 02:30 on the US transition day), the fire must
        // roll to next week's occurrence, not report "no fire at all".
        for offset in 0..=14 {
            let day = now.date_naive() + CDuration::days(offset);
            if let Some(w) = want_weekday
                && day.weekday().num_days_from_sunday() != w
            {
                continue;
            }
            let Some(naive) = day.and_hms_opt(hour as u32, minute as u32, 0) else {
                continue;
            };
            let Some(candidate) = now.timezone().from_local_datetime(&naive).earliest() else {
                continue;
            };
            if candidate > now {
                return Some(candidate);
            }
        }
        None
    }
}

pub(crate) struct CronJob {
    pub(crate) label: &'static str,
    args: &'static [&'static str],
    pub(crate) schedule: Schedule,
    pub(crate) desc: &'static str,
}

/// The scheduled jobs, in fire order. dream-pass first (populates the union
/// views), then the digest reads them; the weekly audit runs non-interactively
/// to stage proposals the user later reviews + applies.
pub(crate) const JOBS: &[CronJob] = &[
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
    CronJob {
        label: "com.alcatraz.i-dream-review",
        args: &["review", "--if-pending"],
        schedule: Schedule::Weekly {
            weekday: 1,
            hour: 9,
            minute: 0,
        },
        desc: "weekly review — opens staged proposals if pending (Mon 09:00)",
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

pub(crate) fn plist_path(label: &str) -> Result<PathBuf> {
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
    let now = chrono::Local::now();
    for job in JOBS {
        let path = plist_path(job.label)?;
        println!("{} — {}", job.label, job.desc);
        if let Some(next) = job.schedule.next_fire_after(now) {
            println!("  next fire: {} ({})", next.format("%Y-%m-%d %H:%M"), job.schedule.human());
        }
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

    // ── next_fire_after ──────────────────────────────────────────────
    // Fixed local timestamps; the walk is pure calendar arithmetic, so
    // each case pins one boundary the status surface depends on.

    use chrono::{Local, TimeZone};

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn daily_before_fire_time_fires_same_day() {
        let s = Schedule::Daily { hour: 2, minute: 45 };
        // 2026-07-17 is a Friday; 01:00 is before 02:45.
        let next = s.next_fire_after(local(2026, 7, 17, 1, 0)).unwrap();
        assert_eq!(next, local(2026, 7, 17, 2, 45));
    }

    #[test]
    fn daily_after_fire_time_rolls_to_tomorrow() {
        let s = Schedule::Daily { hour: 2, minute: 45 };
        let next = s.next_fire_after(local(2026, 7, 17, 12, 0)).unwrap();
        assert_eq!(next, local(2026, 7, 18, 2, 45));
    }

    #[test]
    fn daily_exactly_at_fire_time_is_strictly_after() {
        let s = Schedule::Daily { hour: 2, minute: 45 };
        let next = s.next_fire_after(local(2026, 7, 17, 2, 45)).unwrap();
        assert_eq!(next, local(2026, 7, 18, 2, 45));
    }

    #[test]
    fn weekly_wraps_to_next_week_when_day_passed() {
        // Sunday 02:30 audit; asked on Friday → the coming Sunday.
        let s = Schedule::Weekly {
            weekday: 0,
            hour: 2,
            minute: 30,
        };
        let next = s.next_fire_after(local(2026, 7, 17, 12, 0)).unwrap();
        assert_eq!(next, local(2026, 7, 19, 2, 30));
    }

    #[test]
    fn weekly_same_day_but_past_time_wraps_a_full_week() {
        // 2026-07-19 is a Sunday; at 03:00 the 02:30 fire is gone.
        let s = Schedule::Weekly {
            weekday: 0,
            hour: 2,
            minute: 30,
        };
        let next = s.next_fire_after(local(2026, 7, 19, 3, 0)).unwrap();
        assert_eq!(next, local(2026, 7, 26, 2, 30));
    }

    #[test]
    fn weekly_fire_in_dst_gap_rolls_to_next_week_not_none() {
        // US DST starts 2026-03-08 (2nd Sunday of March): 02:00-03:00 does
        // not exist in America/New_York, which swallows the audit job's
        // Sun 02:30 fire. The next fire must be the FOLLOWING Sunday, not
        // None — validator finding, Batch A gate 2026-07-17.
        use chrono_tz::America::New_York;
        let now = New_York.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
        let s = Schedule::Weekly {
            weekday: 0,
            hour: 2,
            minute: 30,
        };
        let next = s.next_fire_after(now).expect("must roll to next week, not None");
        assert_eq!(
            next,
            New_York.with_ymd_and_hms(2026, 3, 15, 2, 30, 0).unwrap()
        );
    }

    #[test]
    fn daily_fire_in_dst_gap_rolls_to_next_day() {
        use chrono_tz::America::New_York;
        let now = New_York.with_ymd_and_hms(2026, 3, 7, 23, 0, 0).unwrap();
        let s = Schedule::Daily { hour: 2, minute: 30 };
        let next = s.next_fire_after(now).expect("must roll past the gap day");
        assert_eq!(
            next,
            New_York.with_ymd_and_hms(2026, 3, 9, 2, 30, 0).unwrap()
        );
    }

    #[test]
    fn schedule_human_phrases() {
        assert_eq!(Schedule::Daily { hour: 3, minute: 0 }.human(), "daily 03:00");
        assert_eq!(
            Schedule::Weekly {
                weekday: 1,
                hour: 9,
                minute: 0
            }
            .human(),
            "Mon 09:00"
        );
    }
}
