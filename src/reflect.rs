//! `i-dream reflect` — does the guidance i-dream injects actually land?
//!
//! This is the audit half of "Claude gets sharper from my past mistakes": the
//! loop closes silently in every session's injected context, and this command
//! lets you *see* whether it's working. It joins the atone mistake log (what
//! keeps happening) against what i-dream surfaces, and reports per recurring
//! pattern whether it's declining since it started being flagged.
//!
//! Outcome-based and honest: a declining pattern is correlation, not proof
//! i-dream caused it — but a pattern that keeps recurring despite being
//! surfaced every session is a clear signal the guidance isn't landing and
//! needs a stronger intervention (a hook, a rule) than a context reminder.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

struct SlugStat {
    max_sev: String,
    total: usize,
    last7: usize,
    prior7: usize,
    last: DateTime<Utc>,
    /// How many SessionStart injections flagged this slug (from the injection
    /// log, if present). 0 when the log hasn't accrued history yet.
    warned: usize,
}

/// Gather recurring (total ≥ 2) mistake patterns, sorted by severity then count,
/// each joined with how often SessionStart flagged it. Shared by the table and
/// the `--json` view so the trend logic stays single-sourced. Returns the stats
/// plus the injection-log path (for the table's empty-log hint).
fn collect_sorted_stats(home: &PathBuf, now: DateTime<Utc>) -> (Vec<(String, SlugStat)>, PathBuf, Rate) {
    let mut rate = Rate { last7: 0, prior7: 0 };
    let mut by_slug: HashMap<String, SlugStat> = HashMap::new();
    let events = home.join(".claude/atone/events.jsonl");
    if let Ok(content) = fs::read_to_string(&events) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(o) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(slug) = o.get("slug").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(ts) = o
                .get("ts")
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc))
            else {
                continue;
            };
            let sev = o
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("S2")
                .to_string();
            let age = now - ts;
            let e = by_slug.entry(slug.to_string()).or_insert_with(|| SlugStat {
                max_sev: sev.clone(),
                total: 0,
                last7: 0,
                prior7: 0,
                last: ts,
                warned: 0,
            });
            e.total += 1;
            if ts > e.last {
                e.last = ts;
            }
            if sev_rank(&sev) > sev_rank(&e.max_sev) {
                e.max_sev = sev;
            }
            if age <= Duration::days(7) {
                e.last7 += 1;
                rate.last7 += 1;
            } else if age <= Duration::days(14) {
                e.prior7 += 1;
                rate.prior7 += 1;
            }
        }
    }

    // Fold in hint deliveries: how often a live hint for this pattern reached a
    // session. The SessionStart atone summary is not a hint, so it is not counted.
    let inj = home.join(".claude/i-dream/would-fire.jsonl");
    let hints: Vec<(String, String)> =
        crate::interventions::load_interventions(&home.join(".claude/i-dream/interventions.json"))
            .unwrap_or_default()
            .into_iter()
            .filter(|i| i.form == "hint")
            .map(|i| (i.id, i.slug))
            .collect();
    if let Ok(content) = fs::read_to_string(&inj) {
        for (slug, n) in hint_deliveries(&hints, &content) {
            if let Some(e) = by_slug.get_mut(&slug) {
                e.warned = n;
            }
        }
    }

    // Recurring patterns only — the ones i-dream actually surfaces.
    let mut stats: Vec<(String, SlugStat)> =
        by_slug.into_iter().filter(|(_, s)| s.total >= 2).collect();
    stats.sort_by(|a, b| {
        sev_rank(&b.1.max_sev)
            .cmp(&sev_rank(&a.1.max_sev))
            .then(b.1.total.cmp(&a.1.total))
    });
    (stats, inj, rate)
}

pub fn render() -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME unset")?);
    let now = Utc::now();
    let (stats, inj, rate) = collect_sorted_stats(&home, now);

    // `now` (UTC) drives the trend math against UTC event timestamps; the
    // header shows the local calendar date the user actually sees.
    println!(
        "  i-dream reflect — is the guidance landing?  ({})",
        Local::now().format("%Y-%m-%d")
    );
    println!();
    if stats.is_empty() {
        println!("  No recurring mistake patterns yet — nothing to audit.");
        return Ok(());
    }

    let has_warn = stats.iter().any(|(_, s)| s.warned > 0);
    if has_warn {
        println!("  pattern                                  sev  total  7d  warned  trend");
    } else {
        println!("  pattern                                  sev  total  7d  trend");
    }
    for (slug, s) in stats.iter().take(15) {
        let trend = trend_label(s.last7, s.prior7, now - s.last, rate);
        if has_warn {
            println!(
                "  {:<40} {:<3}  {:>4}  {:>2}  {:>5}   {trend}",
                trunc(slug, 40),
                s.max_sev,
                s.total,
                s.last7,
                s.warned
            );
        } else {
            println!(
                "  {:<40} {:<3}  {:>4}  {:>2}  {trend}",
                trunc(slug, 40),
                s.max_sev,
                s.total,
                s.last7
            );
        }
    }
    println!();
    println!("  ↓ landing (fewer lately)   → persisting   ↑ worsening   ✓ dormant (none 14d)");
    println!("  ? unclear (under {MIN_TREND_EVENTS} events, or recording itself fell)   warned = hint deliveries");
    println!(
        "  recording: {} atone event(s) in the last 7 days, {} in the 7 before",
        rate.last7, rate.prior7
    );
    if !has_warn {
        println!(
            "  (no hint deliveries yet — the `warned` column appears once a live hint reaches a session; ledger {})",
            inj.display()
        );
    }
    println!("  Persisting/worsening S3 patterns are candidates to graduate from a");
    println!("  context reminder to a hard guard (a hook or a rule).");
    Ok(())
}

/// Machine-readable reflect — consumed by the menu-bar widget so the trend logic
/// lives only here, never re-derived from the raw logs. Emits aggregate
/// landing/worsening/persisting/dormant counts plus the per-pattern rows.
pub fn render_json() -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME unset")?);
    let now = Utc::now();
    let (stats, _inj, rate) = collect_sorted_stats(&home, now);

    let (mut landing, mut worsening, mut persisting, mut dormant, mut unclear) = (0, 0, 0, 0, 0);
    let patterns: Vec<Value> = stats
        .iter()
        .map(|(slug, s)| {
            let trend = trend_word(s, now, rate);
            match trend {
                "landing" => landing += 1,
                "worsening" => worsening += 1,
                "persisting" => persisting += 1,
                "unclear" => unclear += 1,
                _ => dormant += 1,
            }
            serde_json::json!({
                "slug": slug,
                "severity": s.max_sev,
                "total": s.total,
                "last7": s.last7,
                "warned": s.warned,
                "trend": trend,
            })
        })
        .collect();

    let out = serde_json::json!({
        "date": Local::now().format("%Y-%m-%d").to_string(),
        "summary": {
            "total": stats.len(),
            "landing": landing,
            "worsening": worsening,
            "persisting": persisting,
            "dormant": dormant,
            "unclear": unclear,
        },
        "patterns": patterns,
    });
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}

/// How much atone recording happened across ALL patterns in the last 7 days
/// and the 7 before. A pattern going quiet only means something if the owner
/// was still recording at about the same rate.
#[derive(Clone, Copy, Debug)]
struct Rate {
    last7: usize,
    prior7: usize,
}

impl Rate {
    /// Last week's recording relative to the week before (1.0 when unknown).
    fn ratio(self) -> f64 {
        if self.prior7 == 0 { 1.0 } else { self.last7 as f64 / self.prior7 as f64 }
    }
}

/// Fewest events (last 7 days plus the 7 before) a pattern needs before a
/// movement is called a trend. One against zero is noise, not "worsening".
const MIN_TREND_EVENTS: usize = 3;
/// Recording this far below the week before means a quiet pattern may just be
/// an unfiled one, so "landing" is withheld.
const RECORDING_FELL_BELOW: f64 = 0.5;

/// The single source of truth for the trend, as a bare word. Both the glyphed
/// table label and the JSON derive from this, so adding a variant updates both
/// at once, they can't drift apart.
///
/// Counts are compared against the prior week scaled by the overall recording
/// rate. Too few events, or a quiet week while recording itself fell, give
/// "unclear" rather than a verdict.
fn trend_kind(last7: usize, prior7: usize, since_last: Duration, rate: Rate) -> &'static str {
    if since_last > Duration::days(14) {
        return "dormant";
    }
    if last7 + prior7 < MIN_TREND_EVENTS {
        return "unclear";
    }
    let ratio = rate.ratio();
    let expected = prior7 as f64 * ratio;
    let last = last7 as f64;
    if last < expected {
        if ratio < RECORDING_FELL_BELOW { "unclear" } else { "landing" }
    } else if last > expected {
        "worsening"
    } else {
        "persisting"
    }
}

/// Bare trend word for JSON consumers (the widget).
fn trend_word(s: &SlugStat, now: DateTime<Utc>, rate: Rate) -> &'static str {
    trend_kind(s.last7, s.prior7, now - s.last, rate)
}

/// Trend from recurrence: dormant if nothing in 14 days; otherwise compare the
/// last 7 days against the prior 7, normalised by the recording rate.
fn trend_label(last7: usize, prior7: usize, since_last: Duration, rate: Rate) -> &'static str {
    match trend_kind(last7, prior7, since_last, rate) {
        "dormant" => "✓ dormant",
        "worsening" => "↑ worsening",
        "persisting" => "→ persisting",
        "unclear" => "? unclear",
        _ => "↓ landing",
    }
}

/// Delivered hints per pattern: distinct (hint, session) pairs in the would-fire
/// ledger for live hints, joined to a pattern by `hints` = (id, slug). Ids are
/// compared by their 8-hex prefix, like the delivery scan.
fn hint_deliveries(hints: &[(String, String)], ledger: &str) -> HashMap<String, usize> {
    let slug_of: HashMap<String, &str> = hints
        .iter()
        .map(|(id, slug)| (id.chars().take(8).collect::<String>(), slug.as_str()))
        .collect();
    let mut pairs: std::collections::HashSet<(String, String)> = Default::default();
    for v in ledger.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if v.get("state").and_then(|s| s.as_str()) != Some("live") {
            continue;
        }
        let Some(id) = v.get("id").and_then(|i| i.as_str()) else { continue };
        let id: String = id.chars().take(8).collect();
        if slug_of.contains_key(&id) {
            let sid = v.get("sid").and_then(|s| s.as_str()).unwrap_or("").to_string();
            pairs.insert((id, sid));
        }
    }
    let mut out: HashMap<String, usize> = HashMap::new();
    for (id, _) in pairs {
        *out.entry(slug_of[&id].to_string()).or_default() += 1;
    }
    out
}

fn sev_rank(s: &str) -> u8 {
    match s.trim().to_ascii_uppercase().as_str() {
        "S3" => 3,
        "S2" => 2,
        "S1" => 1,
        _ => 0,
    }
}

fn trunc(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > n {
        let t: String = chars[..n - 1].iter().collect();
        format!("{t}…")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEADY: Rate = Rate { last7: 10, prior7: 10 };
    // Atone recording fell from 13 events on one day (10-06) to 3 in the last 7 days.
    const FELL: Rate = Rate { last7: 3, prior7: 13 };

    #[test]
    fn trend_dormant_when_stale() {
        assert_eq!(trend_label(0, 0, Duration::days(20), STEADY), "✓ dormant");
    }

    #[test]
    fn trend_landing_when_quiet_recently_at_a_steady_rate() {
        assert_eq!(trend_label(0, 3, Duration::days(2), STEADY), "↓ landing");
        assert_eq!(trend_label(1, 4, Duration::days(1), STEADY), "↓ landing");
    }

    #[test]
    fn trend_worsening_when_more_recently() {
        assert_eq!(trend_label(4, 1, Duration::days(1), STEADY), "↑ worsening");
    }

    #[test]
    fn trend_persisting_when_flat() {
        assert_eq!(trend_label(2, 2, Duration::days(1), STEADY), "→ persisting");
    }

    #[test]
    fn one_against_zero_is_not_worsening() {
        // added-scope-without-checking-siblings: 1 this week, 0 before.
        assert_eq!(trend_label(1, 0, Duration::days(1), STEADY), "? unclear");
    }

    #[test]
    fn a_quiet_week_is_not_landing_when_recording_fell() {
        // structural-claim-without-reading-code: 0 this week, 3 before, while
        // the owner filed 3 atones against 13.
        assert_eq!(trend_label(0, 3, Duration::days(10), FELL), "? unclear");
    }

    #[test]
    fn a_rise_in_a_pattern_still_shows_when_recording_fell() {
        assert_eq!(trend_label(3, 1, Duration::days(1), FELL), "↑ worsening");
    }

    #[test]
    fn hint_deliveries_count_distinct_hint_session_pairs_per_pattern() {
        let hints = vec![
            ("aaaaaaaa11111111".to_string(), "slug-a".to_string()),
            ("bbbbbbbb22222222".to_string(), "slug-b".to_string()),
        ];
        let ledger = [
            r#"{"id":"aaaaaaaa11111111","sid":"s1","state":"live"}"#,
            r#"{"id":"aaaaaaaa11111111","sid":"s1","state":"live"}"#,
            r#"{"id":"aaaaaaaa11111111","sid":"s2","state":"live"}"#,
            r#"{"id":"bbbbbbbb22222222","sid":"s1","state":"shadow"}"#,
            r#"{"id":"cccccccc33333333","sid":"s1","state":"live"}"#,
        ]
        .join("\n");
        let got = hint_deliveries(&hints, &ledger);
        assert_eq!(got.get("slug-a"), Some(&2));
        assert_eq!(got.get("slug-b"), None);
    }

    #[test]
    fn sev_rank_orders() {
        assert!(sev_rank("S3") > sev_rank("S2"));
        assert_eq!(sev_rank("x"), 0);
    }
}

/// One recurring mistake slug's recent movement, for `status --json`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SlugTrend {
    pub slug: String,
    pub severity: String,
    pub total: usize,
    /// Events in the last seven days and the seven before.
    pub last7: usize,
    pub prior7: usize,
    /// last7 minus prior7: negative is improving.
    pub delta7: i64,
    pub last: DateTime<Utc>,
}

/// Every recurring (total of two or more) atone slug with its 7-day movement,
/// most severe and most frequent first.
pub fn slug_trends(now: DateTime<Utc>) -> Vec<SlugTrend> {
    let Some(home) = dirs::home_dir() else { return vec![] };
    let (stats, _, _) = collect_sorted_stats(&home, now);
    stats
        .into_iter()
        .map(|(slug, s)| SlugTrend {
            slug,
            severity: s.max_sev,
            total: s.total,
            last7: s.last7,
            prior7: s.prior7,
            delta7: s.last7 as i64 - s.prior7 as i64,
            last: s.last,
        })
        .collect()
}
