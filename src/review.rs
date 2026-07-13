//! `i-dream review` — bring the weekly review TO you.
//!
//! The audit (Sun 02:30 cron) stages proposals + sets a "pending" flag. This
//! command is the *presentation* side, decoupled from generation: it opens a
//! Ghostty window running a fresh `claude` session in the i-dream repo, seeded
//! with a prompt to walk you through the staged proposals. A Monday-09:00
//! LaunchAgent runs `--if-pending` so it surfaces on its own (you never log
//! out, so a calendar time, not login, is the trigger); you can also run it by
//! hand any time. `--add-calendar` drops a recurring event in Calendar.app.

use anyhow::{Context, Result};
use chrono::{Datelike, Duration, Local, Timelike, Utc};
use std::path::PathBuf;
use std::process::Command;

const GHOSTTY_APP: &str = "/Applications/Ghostty.app";

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("HOME").context("HOME unset")?))
}

fn flag_path() -> Result<PathBuf> {
    Ok(home()?.join(".claude/i-dream/.review-pending"))
}

/// Called by the audit's non-interactive path to mark that proposals are
/// waiting. The flag's body is the audit date, so `review` can name it.
pub fn mark_pending(audit_date: &str) -> Result<()> {
    let p = flag_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, audit_date)?;
    Ok(())
}

/// Clear the pending flag — called when an interactive `audit run` completes,
/// i.e. the staged proposals were actually reviewed, not merely shown.
pub fn clear_pending() -> Result<()> {
    let p = flag_path()?;
    if p.exists() {
        std::fs::remove_file(&p)?;
    }
    Ok(())
}

pub fn handle(if_pending: bool, add_calendar: bool) -> Result<()> {
    if add_calendar {
        return install_calendar_event();
    }

    let flag = flag_path()?;
    let pending = flag.exists();

    // The LaunchAgent calls `--if-pending`: stay silent + do nothing unless the
    // audit actually staged something, so it never opens a window into the void.
    if if_pending && !pending {
        return Ok(());
    }

    // Absolute path only — a tilde inside the single-quoted `cd` below would
    // NOT expand and would silently fail the launch (leaving an empty window).
    // Fall back to $HOME if the repo isn't where we expect, so claude still
    // opens somewhere valid.
    let repo = home()?.join("Code/Claude/i-dream");
    let cd_target = if repo.exists() { repo } else { home()? };
    let cd_target = cd_target.display().to_string();

    let staged = std::fs::read_to_string(&flag)
        .ok()
        .filter(|s| !s.trim().is_empty());

    // Quote-safe: single-quoted, and neither value contains a single quote.
    // Backticks stay literal inside single quotes (no command substitution).
    //
    // The apply instruction matters: interactive `i-dream audit run` REGENERATES
    // proposals (fresh LLM call) rather than resuming the staged file, so the
    // review agent must apply approved edits directly and close the loop by
    // hand. The 2026-07-12 review paid for the old wording that said otherwise.
    let prompt = "Run the i-dream weekly review. Read the most recent staged audit \
                  under ~/.claude/i-dream/audits/ (check its header first — a \
                  Reviewed note means it is already actioned, nothing pending) and \
                  the output of `i-dream reflect`, then walk me through each \
                  proposal with your recommendation. Apply the ones I approve by \
                  editing the target files directly — do NOT apply via \
                  `i-dream audit run`; it regenerates a fresh proposal set instead \
                  of resuming the staged one. For each APPLIED proposal that ships \
                  a rule or hook from an insight, also record the graduation \
                  up-vote: find the 1-3 entries in \
                  ~/.claude/subconscious/dreams/patterns.json whose pattern text \
                  states the same lesson (read them — never guess ids), and append \
                  one JSON line per match to \
                  ~/.claude/subconscious/dreams/insight-feedback.jsonl with keys \
                  ts (current UTC ISO), pattern_id, rating set to up, source set \
                  to graduation-manual, and proposal_intent. If no pattern states \
                  the lesson, record nothing and say so. Record each rejection as a line in \
                  ~/.claude/i-dream/audits/_rejections.jsonl with \
                  fp = sha256(expanded_target + newline + lowercased \
                  whitespace-collapsed intent) and a dated reason — verify the fp \
                  recipe by reproducing an existing ledger line first. When done, \
                  append ONE review-outcome line to \
                  ~/.claude/subconscious/dreams/review-outcomes.jsonl shaped \
                  exactly like {\"ts\":\"2026-07-13T09:00:00Z\",\"surfaced\":20,\
                  \"applied\":3,\"source\":\"manual-review\"} — ts MUST be full \
                  RFC3339 with time, surfaced = total proposals in the staged \
                  audit, applied = count you applied. Verify your line parses \
                  by reading it back with jq before moving on — this feeds the \
                  graduation-yield SLO. Then \
                  update the audit file header with Reviewed counts and remove the \
                  ~/.claude/i-dream/.review-pending flag (trash, not rm). Start by \
                  summarizing what is pending.";
    let inner = format!("cd '{cd_target}' && claude '{prompt}'");

    // Launch a *fresh* Ghostty instance, not a window off the running one.
    //
    // `ghostty -e <cmd>` (binary-direct) hands the command to the already-running
    // single instance, which then opens a window inheriting that instance's tab
    // group — so the review window comes up with the prior window's (empty) tabs
    // plus one tab running the review. This is single-instance tab inheritance —
    // NOT macOS saved-state restoration (none exists) and NOT
    // AppleWindowTabbingMode=always. `open -n` forces a new application process,
    // which starts with a clean single-tab window and ignores the running
    // instance's tab group. argv is passed element-by-element (no shell), so the
    // single-quoted `inner` needs no extra escaping here.
    Command::new("open")
        .args(["-n", "-a", GHOSTTY_APP, "--args", "-e", "bash", "-lc", &inner])
        .spawn()
        .with_context(|| format!("Cannot launch Ghostty ({GHOSTTY_APP})"))?;

    // Intentionally do NOT clear the flag here: opening a window is not the same
    // as reviewing. The flag clears when the review actually completes — the
    // seeded prompt tells the review agent to remove it at the end, and an
    // interactive `audit run` also clears it — so the Monday LaunchAgent keeps
    // re-surfacing pending proposals until they're actually handled, and a
    // failed launch never silently consumes them.
    match staged {
        Some(d) => println!("✓ opened the weekly review (proposals staged {}).", d.trim()),
        None => println!("✓ opened the weekly review in a new Ghostty window."),
    }
    println!("  Manual re-open any time:  i-dream review");
    Ok(())
}

/// Write a recurring weekly .ics and `open` it so Calendar.app offers to add
/// it. Using an .ics (vs AppleScript automation) keeps it user-consented and
/// avoids the automation-permission dance — Calendar just shows an "Add" sheet.
fn install_calendar_event() -> Result<()> {
    // Next Monday at 09:00 local — floating local time (no TZID) so it stays at
    // 9am wherever the laptop is.
    let today = Local::now();
    let days_until_mon = (8 - today.weekday().number_from_monday() as i64) % 7;
    let days_until_mon = if days_until_mon == 0 { 7 } else { days_until_mon };
    let start = (today + Duration::days(days_until_mon))
        .with_hour(9)
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .context("could not build start time")?;
    let dt = start.format("%Y%m%dT%H%M%S").to_string();
    // DTSTAMP is the creation instant in UTC (RFC 5545); DTSTART stays floating
    // local so the 09:00 holds wherever the laptop is.
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();

    let ics = format!(
        "BEGIN:VCALENDAR\r\n\
         VERSION:2.0\r\n\
         PRODID:-//i-dream//weekly-review//EN\r\n\
         BEGIN:VEVENT\r\n\
         UID:i-dream-weekly-review@local\r\n\
         DTSTAMP:{stamp}\r\n\
         DTSTART:{dt}\r\n\
         DURATION:PT30M\r\n\
         RRULE:FREQ=WEEKLY;BYDAY=MO\r\n\
         SUMMARY:i-dream weekly review\r\n\
         DESCRIPTION:Review last week's dreamt proposals + GCC changes. It auto-opens \
         Monday 09:00 if proposals are pending. Open any time by running: i-dream review\r\n\
         BEGIN:VALARM\r\n\
         ACTION:DISPLAY\r\n\
         DESCRIPTION:i-dream weekly review\r\n\
         TRIGGER:PT0S\r\n\
         END:VALARM\r\n\
         END:VEVENT\r\n\
         END:VCALENDAR\r\n"
    );

    let path = std::env::temp_dir().join("i-dream-weekly-review.ics");
    std::fs::write(&path, ics).with_context(|| format!("Cannot write {}", path.display()))?;
    Command::new("open")
        .arg(&path)
        .spawn()
        .context("Cannot open the .ics in Calendar")?;

    println!("✓ Calendar.app should now offer to add a recurring event:");
    println!("    “i-dream weekly review” — Mondays 09:00, weekly");
    println!();
    println!("  To open the review manually any time:");
    println!("    i-dream review");
    println!();
    println!("  It also auto-opens Monday 09:00 when proposals are pending");
    println!("  (the `i-dream cron install` review job).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_path_under_idream() {
        // Don't assert the home prefix (varies by machine) — just the tail.
        let p = flag_path().unwrap();
        assert!(p.ends_with(".claude/i-dream/.review-pending"));
    }
}
