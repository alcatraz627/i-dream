//! The `verify` verb: is i-dream producing what it exists to produce?
//!
//! `status` reports whether each part is alive. This reports whether the
//! pipeline's OUTCOMES are happening (patterns, associations, insights,
//! delivery into sessions, the weekly review being answered), one row per
//! stage, as defined in docs/35-outcome-contract.md. It reads only files
//! i-dream and Claude Code already write, and changes nothing.
//!
//! The journal-backed rows are pure functions of the journal and a point in
//! time, so `--history` can replay them across every day the journal covers.
//! That replay is how the check itself was proven: it must fail from
//! 2026-10-06, the day REM and Wake stopped running.

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::io::BufRead;
use std::path::Path;

use crate::interventions::Intervention;
use crate::modules::dreaming::DreamEntry;
use crate::reader::ReaderState;

const CYCLE_MAX_AGE_HOURS: i64 = 36;
const PHASE_WINDOW_CYCLES: usize = 3;
const QUEUE_MAX_HOURS: f64 = 48.0;
const DELIVERY_WINDOW_DAYS: i64 = 7;
const READER_MAX_WAIT_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub row: &'static str,
    pub verdict: Verdict,
    /// What was measured, in words a person can act on.
    pub detail: String,
}

impl Check {
    fn new(row: &'static str, verdict: Verdict, detail: impl Into<String>) -> Self {
        Check { row, verdict, detail: detail.into() }
    }
}

#[derive(Serialize)]
pub struct Report {
    pub as_of: DateTime<Utc>,
    pub checks: Vec<Check>,
}

impl Report {
    /// True when no row failed. UNKNOWN rows do not fail the report, but they
    /// are never shown as passing either.
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.verdict != Verdict::Fail)
    }
}

/// The rows that depend only on the dream journal, evaluated as of `as_of`:
/// entries after that moment are ignored, so the same function serves both
/// today's check and the historical replay.
pub fn journal_checks(entries: &[DreamEntry], as_of: DateTime<Utc>) -> Vec<Check> {
    let mut seen: Vec<&DreamEntry> = entries.iter().filter(|e| e.timestamp <= as_of).collect();
    seen.sort_by_key(|e| e.timestamp);
    let Some(newest) = seen.last().copied() else {
        return ["cycle-ran", "sws-patterns", "rem-associations", "wake-insights", "cycle-budget", "ingest-queue"]
            .into_iter()
            .map(|r| Check::new(r, Verdict::Unknown, "no dream cycle recorded yet"))
            .collect();
    };
    let mut out = Vec::new();

    let age = as_of - newest.timestamp;
    out.push(Check::new(
        "cycle-ran",
        if age <= Duration::hours(CYCLE_MAX_AGE_HOURS) { Verdict::Pass } else { Verdict::Fail },
        format!("newest cycle {}h ago (limit {CYCLE_MAX_AGE_HOURS}h)", age.num_hours()),
    ));

    // Same three-cycle window as REM and Wake, counted over cycles that had
    // sessions to read: one quiet night is normal, three are not.
    let analyzing: Vec<&DreamEntry> = seen
        .iter()
        .rev()
        .filter(|e| e.sessions_analyzed > 0)
        .take(PHASE_WINDOW_CYCLES)
        .copied()
        .collect();
    let patterns: u64 = analyzing.iter().map(|e| e.patterns_extracted).sum();
    out.push(Check::new(
        "sws-patterns",
        if patterns > 0 {
            Verdict::Pass
        } else if analyzing.len() == PHASE_WINDOW_CYCLES {
            Verdict::Fail
        } else {
            Verdict::Unknown
        },
        format!("{patterns} pattern(s) over the last {} cycle(s) that read sessions", analyzing.len()),
    ));

    let window: Vec<&DreamEntry> = seen.iter().rev().take(PHASE_WINDOW_CYCLES).copied().collect();
    let full_window = window.len() == PHASE_WINDOW_CYCLES;
    let phase_row = |row: &'static str, what: &str, total: u64| {
        let verdict = if total > 0 {
            Verdict::Pass
        } else if full_window {
            Verdict::Fail
        } else {
            Verdict::Unknown
        };
        Check::new(row, verdict, format!("{total} {what} over the last {} cycle(s)", window.len()))
    };
    out.push(phase_row(
        "rem-associations",
        "association(s)",
        window.iter().map(|e| e.associations_found).sum(),
    ));
    out.push(phase_row(
        "wake-insights",
        "insight(s) promoted",
        window.iter().map(|e| e.insights_promoted).sum(),
    ));

    out.push(match &newest.assay {
        Some(a) => Check::new(
            "cycle-budget",
            if a.budget_ratio <= 1.0 { Verdict::Pass } else { Verdict::Fail },
            format!(
                "newest cycle spent {:.2}x its budget ({} tokens); over 1.0 starves the later phases",
                a.budget_ratio, newest.tokens_used
            ),
        ),
        None => Check::new("cycle-budget", Verdict::Unknown, "newest cycle has no assay panel"),
    });

    out.push(match &newest.assay {
        Some(a) => Check::new(
            "ingest-queue",
            if a.queue_oldest_hours < QUEUE_MAX_HOURS { Verdict::Pass } else { Verdict::Fail },
            format!(
                "{} item(s) queued, oldest {:.0}h (limit {QUEUE_MAX_HOURS:.0}h)",
                a.queue_depth, a.queue_oldest_hours
            ),
        ),
        None => Check::new("ingest-queue", Verdict::Unknown, "newest cycle has no assay panel"),
    });

    out
}

/// The budget row, from the newest extractor run: dreaming, metacog and
/// introspection together. `None` when no run has been logged, so the
/// journal's dreaming-only row stands in.
pub fn extractor_budget_check(ledger: &str) -> Option<Check> {
    let newest = ledger
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .last()?;
    let tokens = newest.get("tokens")?.as_u64()?;
    let budget = newest.get("budget")?.as_u64()?.max(1);
    let ratio = tokens as f64 / budget as f64;
    Some(Check::new(
        "cycle-budget",
        if ratio <= 1.0 { Verdict::Pass } else { Verdict::Fail },
        format!("newest extractor run spent {ratio:.2}x its budget ({tokens} of {budget} tokens, all modules)"),
    ))
}

/// The cycle-ran row from the extractor ledger. The daily run logs a row even
/// when dreaming skips a quiet day, so a run inside the limit is a cycle that
/// ran. `None` when no run has been logged.
pub fn extractor_ran_check(ledger: &str, as_of: DateTime<Utc>) -> Option<Check> {
    let newest: DateTime<Utc> = ledger
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("ts")?.as_str()?.parse().ok())
        .max()?;
    let age = as_of - newest;
    Some(Check::new(
        "cycle-ran",
        if age <= Duration::hours(CYCLE_MAX_AGE_HOURS) { Verdict::Pass } else { Verdict::Fail },
        format!("newest daily run {}h ago (limit {CYCLE_MAX_AGE_HOURS}h)", age.num_hours()),
    ))
}

/// The ingest-queue row from the queue folder as it is now.
pub fn queue_check(depth: usize, oldest_hours: Option<f64>) -> Check {
    let oldest = oldest_hours.unwrap_or(0.0);
    Check::new(
        "ingest-queue",
        if oldest < QUEUE_MAX_HOURS { Verdict::Pass } else { Verdict::Fail },
        format!("{depth} item(s) queued, oldest {oldest:.0}h (limit {QUEUE_MAX_HOURS:.0}h)"),
    )
}

/// Do the status counter and the delivery rows tell the same story? A zero
/// fired count beside delivered items means one of them reads its data wrong.
pub fn delivery_agrees_check(fired_7d: usize, delivered: usize) -> Check {
    let agree = (fired_7d == 0) == (delivered == 0);
    Check::new(
        "delivery-agrees",
        if agree { Verdict::Pass } else { Verdict::Fail },
        format!("status counts {fired_7d} fired; transcripts show {delivered} live item(s) delivered"),
    )
}

/// Every metacog call in the last week sent at least one unit. `audits` is
/// (timestamp, units analyzed) per audit file.
pub fn metacog_calls_check(audits: &[(&str, u64)], as_of: DateTime<Utc>) -> Check {
    let recent: Vec<u64> = audits
        .iter()
        .filter_map(|(ts, n)| Some((ts.parse::<DateTime<Utc>>().ok()?, *n)))
        .filter(|(t, _)| as_of - *t <= Duration::days(DELIVERY_WINDOW_DAYS))
        .map(|(_, n)| n)
        .collect();
    let empty = recent.iter().filter(|n| **n == 0).count();
    Check::new(
        "metacog-calls",
        if empty == 0 { Verdict::Pass } else { Verdict::Fail },
        format!("{empty} of {} metacog call(s) in the last {DELIVERY_WINDOW_DAYS} days sent no units", recent.len()),
    )
}

/// When live items reached sessions, the firing scan must have judged some of
/// them: zero ledger rows means the scan is not joining what was shown.
pub fn firing_scan_check(shown_pairs: usize, ledger_rows: usize) -> Check {
    if shown_pairs == 0 {
        return Check::new("firing-scan", Verdict::Unknown, "no live item was shown in the window");
    }
    Check::new(
        "firing-scan",
        if ledger_rows > 0 { Verdict::Pass } else { Verdict::Fail },
        format!("{shown_pairs} hint/nudge-session pair(s) shown; firing ledger judged {ledger_rows} of them"),
    )
}

/// Delivery rows: did live nudges and hints actually reach a session?
/// `delivered` holds the 8-hex id prefixes found in hook attachment records;
/// `None` means the transcripts could not be read, which is UNKNOWN, not FAIL.
pub fn delivery_checks(items: &[Intervention], delivered: Option<&HashSet<String>>) -> Vec<Check> {
    let Some(delivered) = delivered else {
        return ["nudge-delivery", "hint-delivery"]
            .into_iter()
            .map(|r| Check::new(r, Verdict::Unknown, "session transcripts could not be read"))
            .collect();
    };
    let row = |name: &'static str, form: &str| {
        let live: Vec<String> = items
            .iter()
            .filter(|i| i.form == form && i.state == "live")
            .map(|i| i.id.chars().take(8).collect())
            .collect();
        if live.is_empty() {
            return Check::new(name, Verdict::Unknown, format!("no live {form}s to deliver"));
        }
        let reached = live.iter().filter(|id| delivered.contains(*id)).count();
        Check::new(
            name,
            if reached > 0 { Verdict::Pass } else { Verdict::Fail },
            format!(
                "{reached} of {} live {form}s reached a session in the last {DELIVERY_WINDOW_DAYS} days",
                live.len()
            ),
        )
    };
    vec![row("nudge-delivery", "nudge"), row("hint-delivery", "hint")]
}

/// The weekly review only moves when its page is answered: the next run
/// refuses to write a new page over an unanswered one.
pub fn reader_check(state: Option<&ReaderState>, now: DateTime<Utc>) -> Check {
    let Some(s) = state else {
        return Check::new("reader-answered", Verdict::Unknown, "reader state not found");
    };
    if s.pending_pages.is_empty() {
        return Check::new("reader-answered", Verdict::Pass, "no page waiting on the owner");
    }
    // A later weekly run moves last_weekly without answering older pages, so
    // the age comes from the oldest page's own week; last_weekly is only the
    // fallback for a slug that does not name a week.
    let oldest = s.pending_pages.iter().filter_map(|p| page_week_start(p)).min().or(s.last_weekly);
    let Some(since) = oldest else {
        return Check::new("reader-answered", Verdict::Unknown, "pages pending but no date for them");
    };
    let days = (now - since).num_days();
    Check::new(
        "reader-answered",
        if days <= READER_MAX_WAIT_DAYS { Verdict::Pass } else { Verdict::Fail },
        format!(
            "{} waiting {days} day(s) on the owner (limit {READER_MAX_WAIT_DAYS})",
            s.pending_pages.join(", ")
        ),
    )
}

/// The earliest moment a page for `idream-reader-<year>w<week>` can exist:
/// the Monday of that ISO week. The weekly run lands mid-week, so this errs
/// toward calling a page old a day or two early, never late.
fn page_week_start(slug: &str) -> Option<DateTime<Utc>> {
    let (year, week) = slug.rsplit_once('-')?.1.split_once('w')?;
    let d = NaiveDate::from_isoywd_opt(year.parse().ok()?, week.parse().ok()?, chrono::Weekday::Mon)?;
    Some(d.and_hms_opt(0, 0, 0)?.and_utc())
}

/// Not measured yet: needs per-slug weekly counts before and after each
/// landed fix (docs/34 stage 3). Reported so its absence stays visible.
pub fn effect_check() -> Check {
    Check::new(
        "effect",
        Verdict::Unknown,
        "not measured: no before/after mistake counts per landed fix yet (docs/34 stage 3)",
    )
}

/// Scan session transcripts modified within the window for `[i-dream:<id>]`
/// tags inside hook attachment records. Text quoted in conversation is not a
/// delivery, so only lines whose record type is `attachment` count.
/// Sub-agent transcripts (`<session>/subagents/*.jsonl`) are included: sub-agents
/// receive nudges too. Returns `None` when the projects folder cannot be read.
pub fn scan_delivered(projects: &Path, now: DateTime<Utc>) -> Option<HashSet<String>> {
    let since = now - Duration::days(DELIVERY_WINDOW_DAYS);
    let mut found = HashSet::new();
    let mut stack = vec![std::fs::read_dir(projects).ok()?];
    while let Some(entries) = stack.pop() {
        for f in entries.flatten() {
            let p = f.path();
            let Ok(meta) = f.metadata() else { continue };
            if meta.is_dir() {
                if let Ok(sub) = std::fs::read_dir(&p) {
                    stack.push(sub);
                }
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // The file time only skips files untouched in the window; each
            // record is then judged by its own timestamp, because a resumed old
            // session carries old deliveries in a freshly modified file.
            let fresh = meta.modified().map(|t| t >= std::time::SystemTime::from(since)).unwrap_or(false);
            if !fresh {
                continue;
            }
            let Ok(file) = std::fs::File::open(&p) else { continue };
            for line in std::io::BufReader::new(file).split(b'\n').map_while(|l| l.ok()) {
                collect_tags(&line, since, &mut found);
            }
        }
    }
    Some(found)
}

fn collect_tags(line: &[u8], since: DateTime<Utc>, found: &mut HashSet<String>) {
    const TAG: &[u8] = b"[i-dream:";
    if !contains(line, TAG) || !contains(line, b"\"type\":\"attachment\"") {
        return;
    }
    if record_time(line).is_none_or(|t| t < since) {
        return;
    }
    let mut rest = line;
    while let Some(at) = find(rest, TAG) {
        let id = &rest[at + TAG.len()..];
        if id.len() > 8 && id[8] == b']' && id[..8].iter().all(|b| b.is_ascii_hexdigit()) {
            found.insert(String::from_utf8_lossy(&id[..8]).into_owned());
        }
        rest = &rest[at + TAG.len()..];
    }
}

/// The record's own top-level `"timestamp":"…"` value.
fn record_time(line: &[u8]) -> Option<DateTime<Utc>> {
    const KEY: &[u8] = b"\"timestamp\":\"";
    let start = find(line, KEY)? + KEY.len();
    let len = line[start..].iter().position(|b| *b == b'"')?;
    std::str::from_utf8(&line[start..start + len]).ok()?.parse().ok()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle).is_some()
}

/// The parsed journal plus how many lines would not parse. Dropped lines are
/// reported, because a schema drift that stops the newest row parsing would
/// otherwise leave the rows judging an older, healthier cycle.
fn load_journal(home: &Path) -> (Vec<DreamEntry>, usize) {
    let body = std::fs::read_to_string(home.join(".claude/subconscious/dreams/journal.jsonl")).unwrap_or_default();
    let mut dropped = 0;
    let entries = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).map_err(|_| dropped += 1).ok())
        .collect();
    (entries, dropped)
}

pub fn gather(home: &Path, now: DateTime<Utc>) -> Result<Report> {
    let (journal, dropped) = load_journal(home);
    let mut checks = journal_checks(&journal, now);
    let spend = std::fs::read_to_string(home.join(".claude/subconscious").join(crate::daemon::EXTRACTOR_SPEND_LOG))
        .unwrap_or_default();
    // The journal rows are replayable history; today's run, budget and queue
    // are read from where they live now.
    let queue_dir = home.join(".claude/subconscious/dreams/ingest-queue");
    let depth = std::fs::read_dir(&queue_dir)
        .map(|d| {
            d.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with(['_', '.']))
                .count()
        })
        .unwrap_or(0);
    let oldest = crate::modules::registry::oldest_child_age(&queue_dir).map(|d| d.as_secs_f64() / 3600.0);
    let live = [extractor_ran_check(&spend, now), extractor_budget_check(&spend), Some(queue_check(depth, oldest))];
    for row in live.into_iter().flatten() {
        let journal_row = checks.iter_mut().find(|c| c.row == row.row);
        match journal_row {
            Some(c) if row.row != "cycle-ran" || row.verdict == Verdict::Pass => *c = row,
            _ => {}
        }
    }
    if dropped > 0 {
        checks.push(Check::new(
            "journal-readable",
            Verdict::Unknown,
            format!("{dropped} journal line(s) did not parse; the cycle rows may be judging an older cycle"),
        ));
    }
    // A broken interventions file must not hide every other row.
    match crate::interventions::load_interventions(&home.join(".claude/i-dream/interventions.json")) {
        Ok(items) => {
            let delivered = scan_delivered(&home.join(".claude/projects"), now);
            checks.extend(delivery_checks(&items, delivered.as_ref()));
            let would_fire = std::fs::read_to_string(home.join(".claude/i-dream/would-fire.jsonl")).unwrap_or_default();
            if let Some(d) = &delivered {
                let reached = items
                    .iter()
                    .filter(|i| i.state == "live" && d.contains(&i.id.chars().take(8).collect::<String>()))
                    .count();
                let fired = crate::status::count_fired_since(&would_fire, now - Duration::days(DELIVERY_WINDOW_DAYS));
                checks.push(delivery_agrees_check(fired, reached));
            }
            let window = now - Duration::days(14);
            let shown = crate::status::count_fired_since(&would_fire, window);
            let ledger_rows = std::fs::read_to_string(home.join(".claude/subconscious/dreams/firings.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                // Only rows for hooks' hints and nudges: lesson and intention
                // rows say nothing about whether these are being judged.
                .filter(|v| v.get("id").and_then(|i| i.as_str()).is_some_and(|i| i.starts_with("N:")))
                .filter(|v| {
                    v.get("ts")
                        .and_then(|t| t.as_str())
                        .and_then(|t| t.parse::<DateTime<Utc>>().ok())
                        .is_some_and(|t| t > window)
                })
                .count();
            checks.push(firing_scan_check(shown, ledger_rows));
        }
        Err(e) => checks.extend(["nudge-delivery", "hint-delivery"].into_iter().map(|r| {
            Check::new(r, Verdict::Unknown, format!("interventions.json unreadable: {e:#}"))
        })),
    }
    let reader: Option<ReaderState> = std::fs::read_to_string(home.join(".claude/i-dream/reader/state.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    checks.push(reader_check(reader.as_ref(), now));
    let audits: Vec<(String, u64)> = std::fs::read_dir(home.join(".claude/subconscious/metacog/audits"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .filter_map(|v| Some((v.get("timestamp")?.as_str()?.to_string(), v.get("units_analyzed")?.as_u64()?)))
                .collect()
        })
        .unwrap_or_default();
    let audit_refs: Vec<(&str, u64)> = audits.iter().map(|(t, n)| (t.as_str(), *n)).collect();
    checks.push(metacog_calls_check(&audit_refs, now));
    checks.push(effect_check());
    Ok(Report { as_of: now, checks })
}

/// One line per calendar day from the first journal entry through `today`,
/// including days with no cycle (so a gap shows as `cycle-ran`): which
/// replayable rows failed at that day's end.
pub fn history(home: &Path, now: DateTime<Utc>) -> Vec<(NaiveDate, Vec<&'static str>)> {
    history_of(&load_journal(home).0, now)
}

/// Today is judged as of `now`, not its end, so a cycle still due tonight
/// does not read as missed.
fn history_of(entries: &[DreamEntry], now: DateTime<Utc>) -> Vec<(NaiveDate, Vec<&'static str>)> {
    let last = now.date_naive();
    let Some(first) = entries.iter().map(|e| e.timestamp.date_naive()).min() else {
        return vec![];
    };
    first
        .iter_days()
        .take_while(|d| *d <= last)
        .map(|d| {
            let end = d.and_hms_opt(23, 59, 59).unwrap().and_utc().min(now);
            let failed = journal_checks(entries, end)
                .into_iter()
                .filter(|c| c.verdict == Verdict::Fail)
                .map(|c| c.row)
                .collect();
            (d, failed)
        })
        .collect()
}

pub fn render_text(r: &Report) -> String {
    let mut s = String::new();
    for c in &r.checks {
        let mark = match c.verdict {
            Verdict::Pass => "PASS   ",
            Verdict::Fail => "FAIL   ",
            Verdict::Unknown => "UNKNOWN",
        };
        s.push_str(&format!("{mark}  {:<17} {}\n", c.row, c.detail));
    }
    let failed = r.checks.iter().filter(|c| c.verdict == Verdict::Fail).count();
    let unknown = r.checks.iter().filter(|c| c.verdict == Verdict::Unknown).count();
    s.push_str(&format!(
        "\n{} of {} rows failing, {unknown} not measurable (contract: docs/35-outcome-contract.md)\n",
        failed,
        r.checks.len()
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidation::assay::AssayPanel;

    #[test]
    fn delivery_counters_must_agree_across_surfaces() {
        assert_eq!(delivery_agrees_check(0, 15).verdict, Verdict::Fail, "status says none fired, verify says delivered");
        assert_eq!(delivery_agrees_check(12, 15).verdict, Verdict::Pass);
        assert_eq!(delivery_agrees_check(0, 0).verdict, Verdict::Pass);
    }

    #[test]
    fn a_metacog_call_that_sent_nothing_fails() {
        let now: DateTime<Utc> = "2026-10-26T12:00:00Z".parse().unwrap();
        let ok = [("2026-10-25T23:00:00Z", 12u64)];
        assert_eq!(metacog_calls_check(&ok, now).verdict, Verdict::Pass);
        let bad = [("2026-10-24T23:00:00Z", 0u64), ("2026-10-25T23:00:00Z", 12)];
        assert_eq!(metacog_calls_check(&bad, now).verdict, Verdict::Fail);
        let old_bad = [("2026-10-10T23:00:00Z", 0u64)];
        assert_eq!(metacog_calls_check(&old_bad, now).verdict, Verdict::Pass, "outside the window");
    }

    #[test]
    fn firing_scan_must_record_something_when_items_were_shown() {
        assert_eq!(firing_scan_check(40, 0).verdict, Verdict::Fail);
        assert_eq!(firing_scan_check(40, 31).verdict, Verdict::Pass);
        assert_eq!(firing_scan_check(0, 0).verdict, Verdict::Unknown);
    }

    #[test]
    fn a_quiet_day_whose_daily_run_happened_is_not_a_missed_cycle() {
        let now: DateTime<Utc> = "2026-10-26T12:00:00Z".parse().unwrap();
        let ledger = "{\"ts\":\"2026-10-25T23:00:00Z\",\"tokens\":0,\"budget\":10000}\n";
        let c = extractor_ran_check(ledger, now).unwrap();
        assert_eq!((c.row, c.verdict), ("cycle-ran", Verdict::Pass));
        let stale = "{\"ts\":\"2026-10-23T23:00:00Z\",\"tokens\":0,\"budget\":10000}\n";
        assert_eq!(extractor_ran_check(stale, now).unwrap().verdict, Verdict::Fail);
    }

    #[test]
    fn queue_row_reads_the_live_backlog() {
        let c = queue_check(9, Some(67.0));
        assert_eq!((c.row, c.verdict), ("ingest-queue", Verdict::Fail));
        assert!(c.detail.contains("9 item(s)"));
        assert_eq!(queue_check(0, None).verdict, Verdict::Pass);
    }

    #[test]
    fn budget_row_judges_the_newest_extractor_run_across_all_modules() {
        let ledger = "{\"ts\":\"2026-10-24T23:00:00Z\",\"tokens\":9000,\"budget\":10000}\n\
                      {\"ts\":\"2026-10-25T23:00:00Z\",\"tokens\":12000,\"budget\":10000}\n";
        let c = extractor_budget_check(ledger).unwrap();
        assert_eq!(c.row, "cycle-budget");
        assert_eq!(c.verdict, Verdict::Fail, "newest run spent 1.2x");
        assert!(c.detail.contains("12000"));
        assert!(extractor_budget_check("").is_none(), "no ledger falls back to the journal row");
    }

    fn entry(day: u32, sessions: u64, patterns: u64, assoc: u64, insights: u64, ratio: f64, queue_h: f64) -> DreamEntry {
        DreamEntry {
            id: format!("e{day}"),
            timestamp: format!("2026-10-{day:02}T23:00:00Z").parse().unwrap(),
            phase: "all".into(),
            sessions_analyzed: sessions,
            patterns_extracted: patterns,
            associations_found: assoc,
            insights_promoted: insights,
            tokens_used: (ratio * 25_000.0) as u64,
            cycle_id: String::new(),
            assay: Some(AssayPanel {
                dup_rate: 0.1,
                provenance_complete: 1.0,
                budget_ratio: ratio,
                queue_depth: 3,
                queue_oldest_hours: queue_h,
                reactivated_patterns: 0,
                patterns_total: 100,
            }),
        }
    }

    fn at(day: u32) -> DateTime<Utc> {
        format!("2026-10-{day:02}T23:59:59Z").parse().unwrap()
    }

    fn verdict(checks: &[Check], row: &str) -> Verdict {
        checks.iter().find(|c| c.row == row).unwrap().verdict
    }

    /// The real shape of the regression: patterns kept flowing (so a
    /// "produced anything" check stayed green) while REM and Wake stopped.
    fn regression_journal() -> Vec<DreamEntry> {
        vec![
            entry(3, 5, 10, 11, 8, 0.6, 2.0),
            entry(4, 5, 10, 9, 9, 0.5, 2.0),
            entry(5, 5, 10, 12, 7, 0.4, 2.0),
            entry(6, 6, 12, 0, 0, 2.5, 30.0),
            entry(7, 6, 12, 0, 0, 2.5, 60.0),
            entry(8, 6, 12, 0, 0, 2.5, 82.0),
        ]
    }

    #[test]
    fn healthy_cycles_pass_every_journal_row() {
        let c = journal_checks(&regression_journal(), at(5));
        assert!(c.iter().all(|c| c.verdict == Verdict::Pass), "{c:?}");
    }

    #[test]
    fn budget_overrun_fails_on_the_first_bad_day() {
        let c = journal_checks(&regression_journal(), at(6));
        assert_eq!(verdict(&c, "cycle-budget"), Verdict::Fail);
        // One quiet cycle inside a productive window is not yet a failure.
        assert_eq!(verdict(&c, "rem-associations"), Verdict::Pass);
        assert_eq!(verdict(&c, "sws-patterns"), Verdict::Pass);
    }

    #[test]
    fn three_silent_cycles_fail_rem_and_wake_while_sws_stays_green() {
        let c = journal_checks(&regression_journal(), at(8));
        assert_eq!(verdict(&c, "rem-associations"), Verdict::Fail);
        assert_eq!(verdict(&c, "wake-insights"), Verdict::Fail);
        assert_eq!(verdict(&c, "ingest-queue"), Verdict::Fail);
        assert_eq!(verdict(&c, "sws-patterns"), Verdict::Pass);
    }

    #[test]
    fn a_stale_journal_fails_cycle_ran() {
        let c = journal_checks(&regression_journal(), at(12));
        assert_eq!(verdict(&c, "cycle-ran"), Verdict::Fail);
    }

    #[test]
    fn entries_after_as_of_are_ignored() {
        // At day 5 the bad cycles have not happened yet.
        let c = journal_checks(&regression_journal(), at(5));
        assert_eq!(verdict(&c, "cycle-budget"), Verdict::Pass);
    }

    #[test]
    fn one_quiet_sws_night_is_not_a_failure_but_three_are() {
        let mut j = regression_journal();
        j.push(entry(9, 4, 0, 0, 0, 0.5, 1.0));
        assert_eq!(verdict(&journal_checks(&j, at(9)), "sws-patterns"), Verdict::Pass);
        j.push(entry(10, 4, 0, 0, 0, 0.5, 1.0));
        j.push(entry(11, 4, 0, 0, 0, 0.5, 1.0));
        assert_eq!(verdict(&journal_checks(&j, at(11)), "sws-patterns"), Verdict::Fail);
    }

    #[test]
    fn history_covers_gap_days_and_reds_from_the_regression() {
        let mut j = regression_journal();
        j.retain(|e| e.timestamp.format("%d").to_string() != "04");
        j.push(entry(12, 6, 12, 0, 0, 2.5, 90.0));
        let h = history_of(&j, at(14));
        let on = |d: u32| &h.iter().find(|(day, _)| day.format("%d").to_string() == format!("{d:02}")).unwrap().1;
        assert!(on(4).is_empty(), "a one-day gap is inside the 36h limit");
        assert!(on(5).is_empty());
        assert!(on(6).contains(&"cycle-budget"));
        assert!(on(8).contains(&"rem-associations"));
        assert!(on(10).contains(&"cycle-ran"), "two empty days fail cycle-ran");
        assert!(on(14).contains(&"cycle-ran"), "days after the last cycle are replayed too");
        assert_eq!(h.len(), 12, "every calendar day from the 3rd through today, the 14th");
    }

    /// The widget decodes this document: row, lowercase verdict and detail
    /// on every check, plus as_of.
    #[test]
    fn json_report_carries_the_fields_the_widget_reads() {
        let r = Report { as_of: at(5), checks: vec![Check::new("hint-delivery", Verdict::Fail, "0 of 17")] };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v["as_of"].is_string());
        assert_eq!(v["checks"][0]["row"], "hint-delivery");
        assert_eq!(v["checks"][0]["verdict"], "fail");
        assert_eq!(v["checks"][0]["detail"], "0 of 17");
    }

    #[test]
    fn a_failing_row_fails_the_report_and_unknown_does_not() {
        let row = |v| Check::new("x", v, "");
        let report = |vs: Vec<Verdict>| Report { as_of: at(5), checks: vs.into_iter().map(row).collect() };
        assert!(report(vec![Verdict::Pass, Verdict::Unknown]).ok());
        assert!(!report(vec![Verdict::Pass, Verdict::Fail]).ok());
        assert!(!report(vec![Verdict::Unknown, Verdict::Fail]).ok());
    }

    #[test]
    fn empty_or_short_journal_is_unknown_not_pass() {
        let c = journal_checks(&[], at(5));
        assert!(c.iter().all(|c| c.verdict == Verdict::Unknown));
        let one = vec![entry(5, 1, 1, 0, 0, 0.5, 1.0)];
        let c = journal_checks(&one, at(5));
        assert_eq!(verdict(&c, "rem-associations"), Verdict::Unknown);
    }

    #[test]
    fn missing_assay_is_unknown() {
        let mut e = entry(5, 1, 1, 1, 1, 0.5, 1.0);
        e.assay = None;
        let c = journal_checks(&[e], at(5));
        assert_eq!(verdict(&c, "cycle-budget"), Verdict::Unknown);
        assert_eq!(verdict(&c, "ingest-queue"), Verdict::Unknown);
    }

    fn item(id: &str, form: &str, state: &str) -> Intervention {
        serde_json::from_value(serde_json::json!({
            "id": id, "form": form, "state": state, "slug": "s", "body": "b",
            "source": "atone-precheck", "trigger": {}, "created": "2026-10-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn delivery_fails_when_no_live_hint_reached_a_session() {
        let items = vec![item("aaaaaaaa11", "nudge", "live"), item("bbbbbbbb22", "hint", "live")];
        let delivered: HashSet<String> = ["aaaaaaaa".to_string()].into();
        let c = delivery_checks(&items, Some(&delivered));
        assert_eq!(verdict(&c, "nudge-delivery"), Verdict::Pass);
        assert_eq!(verdict(&c, "hint-delivery"), Verdict::Fail);
        let c = delivery_checks(&items, None);
        assert_eq!(verdict(&c, "hint-delivery"), Verdict::Unknown, "unreadable transcripts are not a failure");
    }

    #[test]
    fn delivery_ignores_non_live_items() {
        let items = vec![item("cccccccc33", "hint", "shadow")];
        let delivered: HashSet<String> = ["cccccccc".to_string()].into();
        let c = delivery_checks(&items, Some(&delivered));
        assert_eq!(verdict(&c, "hint-delivery"), Verdict::Unknown);
    }

    const NEW: &str = r#""timestamp":"2026-10-08T10:00:00Z""#;

    #[test]
    fn only_attachment_records_count_as_delivery() {
        let since = at(1);
        let mut found = HashSet::new();
        let line = |body: &str| format!(r#"{{{NEW},{body}}}"#).into_bytes();
        collect_tags(&line(r#""type":"assistant","text":"see [i-dream:deadbeef] here""#), since, &mut found);
        assert!(found.is_empty(), "a quoted tag is not a delivery");
        collect_tags(&line(r#""type":"attachment","x":"[i-dream:deadbeef] b [i-dream:0123abcd] b""#), since, &mut found);
        assert_eq!(found.len(), 2);
        collect_tags(&line(r#""type":"attachment","x":"[i-dream:nothex!!] [i-dream:abc]""#), since, &mut found);
        assert_eq!(found.len(), 2, "malformed tags are skipped");
    }

    #[test]
    fn an_old_record_in_a_fresh_file_is_not_a_recent_delivery() {
        let mut found = HashSet::new();
        let old = br#"{"timestamp":"2026-09-01T10:00:00Z","type":"attachment","x":"[i-dream:deadbeef]"}"#;
        collect_tags(old, at(1), &mut found);
        assert!(found.is_empty());
        let undated = br#"{"type":"attachment","x":"[i-dream:deadbeef]"}"#;
        collect_tags(undated, at(1), &mut found);
        assert!(found.is_empty(), "a record with no time cannot prove a recent delivery");
    }

    #[test]
    fn scan_reads_subagent_transcripts_and_reports_a_missing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("proj/sess/subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let rec = format!(r#"{{"timestamp":"{}","type":"attachment","x":"[i-dream:feedf00d]"}}"#, Utc::now().to_rfc3339());
        std::fs::write(sub.join("agent-1.jsonl"), rec).unwrap();
        let found = scan_delivered(dir.path(), Utc::now()).unwrap();
        assert!(found.contains("feedf00d"), "{found:?}");
        assert!(scan_delivered(&dir.path().join("absent"), Utc::now()).is_none());
    }

    #[test]
    fn a_broken_interventions_file_leaves_the_other_rows_standing() {
        let home = tempfile::tempdir().unwrap();
        let d = home.path().join(".claude/i-dream");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("interventions.json"), "{broken").unwrap();
        let r = gather(home.path(), at(5)).unwrap();
        assert_eq!(verdict(&r.checks, "hint-delivery"), Verdict::Unknown);
        assert!(r.checks.iter().any(|c| c.row == "cycle-ran"), "journal rows still reported");
    }

    #[test]
    fn reader_page_waiting_too_long_fails() {
        let mut s = ReaderState::default();
        assert_eq!(reader_check(Some(&s), at(20)).verdict, Verdict::Pass);
        // 2026w42 starts Monday 2026-10-12.
        s.pending_pages = vec!["idream-reader-2026w42".into()];
        s.last_weekly = Some(at(13));
        assert_eq!(reader_check(Some(&s), at(19)).verdict, Verdict::Pass);
        assert_eq!(reader_check(Some(&s), at(21)).verdict, Verdict::Fail);
        assert_eq!(reader_check(None, at(21)).verdict, Verdict::Unknown);
    }

    #[test]
    fn a_later_weekly_run_does_not_make_an_old_page_young() {
        let s = ReaderState {
            pending_pages: vec!["idream-reader-2026w41".into(), "idream-reader-2026w43".into()],
            last_weekly: Some(at(20)),
            ..Default::default()
        };
        assert_eq!(reader_check(Some(&s), at(21)).verdict, Verdict::Fail, "w41 began 2026-10-05");
    }
}
