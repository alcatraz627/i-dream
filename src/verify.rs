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

/// Delivery rows: did live nudges and hints actually reach a session?
/// `delivered` holds the 8-hex id prefixes found in hook attachment records.
pub fn delivery_checks(items: &[Intervention], delivered: &HashSet<String>) -> Vec<Check> {
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
    let Some(since) = s.last_weekly else {
        return Check::new("reader-answered", Verdict::Unknown, "pages pending but no weekly run time");
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
pub fn scan_delivered(projects: &Path, now: DateTime<Utc>) -> HashSet<String> {
    let cutoff = std::time::SystemTime::from(now - Duration::days(DELIVERY_WINDOW_DAYS));
    let mut found = HashSet::new();
    let Ok(dirs) = std::fs::read_dir(projects) else { return found };
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let fresh = f.metadata().and_then(|m| m.modified()).map(|t| t >= cutoff).unwrap_or(false);
            if !fresh {
                continue;
            }
            let Ok(file) = std::fs::File::open(&p) else { continue };
            for line in std::io::BufReader::new(file).split(b'\n').map_while(|l| l.ok()) {
                collect_tags(&line, &mut found);
            }
        }
    }
    found
}

fn collect_tags(line: &[u8], found: &mut HashSet<String>) {
    const TAG: &[u8] = b"[i-dream:";
    if !contains(line, TAG) || !contains(line, b"\"type\":\"attachment\"") {
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

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle).is_some()
}

fn load_journal(home: &Path) -> Vec<DreamEntry> {
    std::fs::read_to_string(home.join(".claude/subconscious/dreams/journal.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn gather(home: &Path, now: DateTime<Utc>) -> Result<Report> {
    let mut checks = journal_checks(&load_journal(home), now);
    let items = crate::interventions::load_interventions(&home.join(".claude/i-dream/interventions.json"))?;
    let delivered = scan_delivered(&home.join(".claude/projects"), now);
    checks.extend(delivery_checks(&items, &delivered));
    let reader: Option<ReaderState> = std::fs::read_to_string(home.join(".claude/i-dream/reader/state.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    checks.push(reader_check(reader.as_ref(), now));
    checks.push(effect_check());
    Ok(Report { as_of: now, checks })
}

/// One line per calendar day the journal spans, including days with no
/// cycle (so a gap shows as `cycle-ran`): which replayable rows failed at
/// that day's end.
pub fn history(home: &Path) -> Vec<(NaiveDate, Vec<&'static str>)> {
    history_of(&load_journal(home))
}

fn history_of(entries: &[DreamEntry]) -> Vec<(NaiveDate, Vec<&'static str>)> {
    let (Some(first), Some(last)) = (
        entries.iter().map(|e| e.timestamp.date_naive()).min(),
        entries.iter().map(|e| e.timestamp.date_naive()).max(),
    ) else {
        return vec![];
    };
    first
        .iter_days()
        .take_while(|d| *d <= last)
        .map(|d| {
            let end = d.and_hms_opt(23, 59, 59).unwrap().and_utc();
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
        let h = history_of(&j);
        let on = |d: u32| &h.iter().find(|(day, _)| day.format("%d").to_string() == format!("{d:02}")).unwrap().1;
        assert!(on(4).is_empty(), "a one-day gap is inside the 36h limit");
        assert!(on(5).is_empty());
        assert!(on(6).contains(&"cycle-budget"));
        assert!(on(8).contains(&"rem-associations"));
        assert!(on(10).contains(&"cycle-ran"), "two empty days fail cycle-ran");
        assert_eq!(h.len(), 10, "every calendar day from the 3rd to the 12th");
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
        let c = delivery_checks(&items, &delivered);
        assert_eq!(verdict(&c, "nudge-delivery"), Verdict::Pass);
        assert_eq!(verdict(&c, "hint-delivery"), Verdict::Fail);
    }

    #[test]
    fn delivery_ignores_non_live_items() {
        let items = vec![item("cccccccc33", "hint", "shadow")];
        let delivered: HashSet<String> = ["cccccccc".to_string()].into();
        let c = delivery_checks(&items, &delivered);
        assert_eq!(verdict(&c, "hint-delivery"), Verdict::Unknown);
    }

    #[test]
    fn only_attachment_records_count_as_delivery() {
        let mut found = HashSet::new();
        collect_tags(br#"{"type":"assistant","text":"see [i-dream:deadbeef] here"}"#, &mut found);
        assert!(found.is_empty(), "a quoted tag is not a delivery");
        collect_tags(br#"{"type":"attachment","x":"[i-dream:deadbeef] body [i-dream:0123abcd] b"}"#, &mut found);
        assert_eq!(found.len(), 2);
        collect_tags(br#"{"type":"attachment","x":"[i-dream:nothex!!] [i-dream:abc]"}"#, &mut found);
        assert_eq!(found.len(), 2, "malformed tags are skipped");
    }

    #[test]
    fn reader_page_waiting_too_long_fails() {
        let mut s = ReaderState::default();
        assert_eq!(reader_check(Some(&s), at(20)).verdict, Verdict::Pass);
        s.pending_pages = vec!["idream-reader-2026w42".into()];
        s.last_weekly = Some(at(13));
        assert_eq!(reader_check(Some(&s), at(19)).verdict, Verdict::Pass);
        assert_eq!(reader_check(Some(&s), at(21)).verdict, Verdict::Fail);
        assert_eq!(reader_check(None, at(21)).verdict, Verdict::Unknown);
    }
}
