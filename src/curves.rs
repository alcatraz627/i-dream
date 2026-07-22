//! Recurrence curves — the system's first true efficacy readout (A3).
//!
//! For each mistake slug in the atone ledger, this buckets recurrences by
//! ISO week and writes `~/.claude/i-dream/derived/curves.json` for the
//! assay, the weekly receipt, and the dashboard to trend. A curve that
//! bends after an insight ships is the only outcome evidence the felt-
//! metabolism arc accepts; a curve that stays flat despite injection is a
//! paperwork verdict on that insight.
//!
//! Each slug carries an `interventions` list that is EMPTY today: nothing
//! machine-readable joins a slug to the ship date of the insight/rule/hook
//! targeting it yet. Phase 2's compiled interventions carry slugs and will
//! fill it — fabricating the join earlier would be the dream-pass cadence
//! mistake again (a column invented because the schema had room for it).

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How far back the weekly series reaches.
const WINDOW_WEEKS: i64 = 26;
/// Weeks compared for the trend verdict (last N vs the N before them).
const TREND_SPAN_WEEKS: u32 = 4;

#[derive(Debug, Serialize, Deserialize)]
pub struct CurvesDoc {
    pub schema_version: u32,
    pub generated_at: DateTime<Utc>,
    pub window_weeks: i64,
    /// Total ledger events that fell inside the window.
    pub events_in_window: u64,
    /// Slugs sorted by total recurrence, highest first.
    pub slugs: Vec<SlugCurve>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SlugCurve {
    pub slug: String,
    pub total: u64,
    pub first: NaiveDate,
    pub last: NaiveDate,
    /// ISO week ("2026-W29") → recurrence count; only non-zero weeks appear.
    pub weekly: Vec<WeekCount>,
    /// Last TREND_SPAN_WEEKS vs the span before: "rising" | "flat" | "falling".
    pub trend: String,
    /// Ship dates of insights/rules/hooks targeting this slug. Empty until
    /// Phase 2's compiled interventions carry slugs — never fabricated.
    pub interventions: Vec<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeekCount {
    pub week: String,
    pub count: u64,
}

/// Parse one atone ledger line to (slug, event date). The date prefers the
/// id's embedded stamp (`mist-YYYYMMDD-…` — present on every event since the
/// ledger began) and falls back to a `ts` field. Undatable or slugless
/// lines are skipped, not guessed.
fn parse_event_line(line: &str) -> Option<(String, NaiveDate)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let slug = v.get("slug")?.as_str()?.trim().to_string();
    if slug.is_empty() {
        return None;
    }
    let from_id = v.get("id").and_then(|i| i.as_str()).and_then(|id| {
        let digits = id.split('-').nth(1)?;
        NaiveDate::parse_from_str(digits, "%Y%m%d").ok()
    });
    let date = from_id.or_else(|| {
        v.get("ts")
            .and_then(|t| t.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc).date_naive())
    })?;
    Some((slug, date))
}

fn iso_week_key(d: NaiveDate) -> String {
    let w = d.iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

/// Build the curves document from dated events. Pure — hermetically testable.
fn compute_curves(events: &[(String, NaiveDate)], now: DateTime<Utc>) -> CurvesDoc {
    let cutoff = (now - chrono::Duration::weeks(WINDOW_WEEKS)).date_naive();
    let trend_split = (now - chrono::Duration::weeks(TREND_SPAN_WEEKS as i64)).date_naive();
    let trend_floor = (now - chrono::Duration::weeks(2 * TREND_SPAN_WEEKS as i64)).date_naive();

    let mut by_slug: HashMap<&str, Vec<NaiveDate>> = HashMap::new();
    for (slug, d) in events {
        if *d >= cutoff {
            by_slug.entry(slug.as_str()).or_default().push(*d);
        }
    }

    let mut slugs: Vec<SlugCurve> = by_slug
        .into_iter()
        .map(|(slug, mut dates)| {
            dates.sort();
            let mut weekly: HashMap<String, u64> = HashMap::new();
            for d in &dates {
                *weekly.entry(iso_week_key(*d)).or_default() += 1;
            }
            let mut weekly: Vec<WeekCount> = weekly
                .into_iter()
                .map(|(week, count)| WeekCount { week, count })
                .collect();
            weekly.sort_by(|a, b| a.week.cmp(&b.week));

            let recent = dates.iter().filter(|d| **d >= trend_split).count();
            let prior = dates
                .iter()
                .filter(|d| **d >= trend_floor && **d < trend_split)
                .count();
            let trend = match (prior, recent) {
                (0, 0) => "flat",
                (0, _) => "rising",
                (p, r) if (r as f64) > p as f64 * 1.25 => "rising",
                (p, r) if (r as f64) < p as f64 * 0.75 => "falling",
                _ => "flat",
            };

            SlugCurve {
                slug: slug.to_string(),
                total: dates.len() as u64,
                first: *dates.first().expect("non-empty by construction"),
                last: *dates.last().expect("non-empty by construction"),
                weekly,
                trend: trend.to_string(),
                interventions: vec![],
            }
        })
        .collect();
    slugs.sort_by(|a, b| b.total.cmp(&a.total).then(a.slug.cmp(&b.slug)));

    CurvesDoc {
        schema_version: 1,
        generated_at: now,
        window_weeks: WINDOW_WEEKS,
        events_in_window: slugs.iter().map(|s| s.total).sum(),
        slugs,
    }
}

/// Read the ledger at `events_path`, compute, and atomically write
/// `curves.json` at `out_path`. Both paths injectable so tests never touch
/// the live ledger.
pub fn compute_and_persist_at(
    events_path: &Path,
    out_path: &Path,
    now: DateTime<Utc>,
) -> Result<CurvesDoc> {
    let body = std::fs::read_to_string(events_path)
        .with_context(|| format!("reading atone ledger at {}", events_path.display()))?;
    let events: Vec<(String, NaiveDate)> = body.lines().filter_map(parse_event_line).collect();
    let doc = compute_curves(&events, now);
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = out_path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&doc)?)?;
    std::fs::rename(&tmp, out_path)?;
    Ok(doc)
}

/// Production entrypoint: the live atone ledger → the derived curves file.
pub fn compute_and_persist() -> Result<(CurvesDoc, PathBuf)> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    let events = home.join(".claude/atone/events.jsonl");
    let out = home.join(".claude/i-dream/derived/curves.json");
    let doc = compute_and_persist_at(&events, &out, Utc::now())?;
    Ok((doc, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 22, 12, 0, 0).unwrap()
    }

    #[test]
    fn parses_id_stamp_ts_fallback_and_skips_garbage() {
        let by_id = r#"{"id":"mist-20260718-101437-d7","slug":"added-scope","severity":"S2"}"#;
        let by_ts = r#"{"slug":"late-slug","ts":"2026-07-01T10:00:00+00:00"}"#;
        let undatable = r#"{"slug":"no-date-anywhere"}"#;
        let slugless = r#"{"id":"mist-20260718-101437-d7"}"#;
        assert_eq!(
            parse_event_line(by_id).unwrap(),
            ("added-scope".into(), NaiveDate::from_ymd_opt(2026, 7, 18).unwrap())
        );
        assert_eq!(
            parse_event_line(by_ts).unwrap().1,
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
        );
        assert!(parse_event_line(undatable).is_none());
        assert!(parse_event_line(slugless).is_none());
        assert!(parse_event_line("not json").is_none());
    }

    #[test]
    fn buckets_weekly_ranks_by_total_and_windows_out_old_events() {
        let d = |m: u32, day: u32| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let events = vec![
            ("busy".to_string(), d(7, 13)),
            ("busy".to_string(), d(7, 14)),
            ("busy".to_string(), d(7, 20)),
            ("quiet".to_string(), d(7, 1)),
            ("ancient".to_string(), d(1, 1)), // outside the 26-week window
        ];
        let doc = compute_curves(&events, now());
        assert_eq!(doc.events_in_window, 4);
        assert_eq!(doc.slugs[0].slug, "busy");
        assert_eq!(doc.slugs[0].total, 3);
        // 07-13/14 share an ISO week; 07-20 starts the next.
        assert_eq!(doc.slugs[0].weekly.len(), 2);
        assert_eq!(doc.slugs[0].weekly[0].count, 2);
        assert!(doc.slugs.iter().all(|s| s.slug != "ancient"));
        assert!(doc.slugs.iter().all(|s| s.interventions.is_empty()));
    }

    #[test]
    fn trend_classifies_rising_falling_and_flat() {
        let d = |m: u32, day: u32| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        // rising: nothing in the prior span, twice in the recent span.
        let rising = vec![("r".to_string(), d(7, 10)), ("r".to_string(), d(7, 18))];
        // falling: twice in the prior span (May 27+ is within 8 weeks of
        // Jul 22), nothing recent.
        let falling = vec![("f".to_string(), d(6, 1)), ("f".to_string(), d(6, 10))];
        // flat: one in each span.
        let flat = vec![("s".to_string(), d(6, 10)), ("s".to_string(), d(7, 10))];
        assert_eq!(compute_curves(&rising, now()).slugs[0].trend, "rising");
        assert_eq!(compute_curves(&falling, now()).slugs[0].trend, "falling");
        assert_eq!(compute_curves(&flat, now()).slugs[0].trend, "flat");
    }

    #[test]
    fn persists_atomically_to_injected_paths() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("events.jsonl");
        std::fs::write(
            &ledger,
            r#"{"id":"mist-20260718-101437-d7","slug":"added-scope"}
{"id":"mist-20260721-121411-c8","slug":"file-referenced-without-full-path"}
garbage line
"#,
        )
        .unwrap();
        let out = dir.path().join("derived/curves.json");
        let doc = compute_and_persist_at(&ledger, &out, now()).unwrap();
        assert_eq!(doc.events_in_window, 2);
        let read_back: CurvesDoc =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(read_back.slugs.len(), 2);
        assert!(!out.with_extension("json.tmp").exists(), "tmp cleaned up");
    }
}
