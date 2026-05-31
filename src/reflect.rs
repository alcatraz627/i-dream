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
fn collect_sorted_stats(home: &PathBuf, now: DateTime<Utc>) -> (Vec<(String, SlugStat)>, PathBuf) {
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
            } else if age <= Duration::days(14) {
                e.prior7 += 1;
            }
        }
    }

    // Fold in the injection log (what SessionStart actually flagged), if present.
    let inj = home.join(".claude/i-dream/injections.jsonl");
    if let Ok(content) = fs::read_to_string(&inj) {
        for line in content.lines() {
            let Ok(o) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if let Some(slugs) = o.get("slugs").and_then(|v| v.as_array()) {
                for s in slugs.iter().filter_map(|v| v.as_str()) {
                    if let Some(e) = by_slug.get_mut(s) {
                        e.warned += 1;
                    }
                }
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
    (stats, inj)
}

pub fn render() -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME unset")?);
    let now = Utc::now();
    let (stats, inj) = collect_sorted_stats(&home, now);

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
        let trend = trend_label(s.last7, s.prior7, now - s.last);
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
    if !has_warn {
        println!(
            "  (injection log empty — `warned` column appears once SessionStart history accrues at {})",
            inj.display()
        );
    }
    println!("  Persisting/worsening S3 patterns are candidates to graduate from a");
    println!("  context reminder to a hard guard — `i-dream audit run`.");
    Ok(())
}

/// Machine-readable reflect — consumed by the menu-bar widget so the trend logic
/// lives only here, never re-derived from the raw logs. Emits aggregate
/// landing/worsening/persisting/dormant counts plus the per-pattern rows.
pub fn render_json() -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME unset")?);
    let now = Utc::now();
    let (stats, _inj) = collect_sorted_stats(&home, now);

    let (mut landing, mut worsening, mut persisting, mut dormant) = (0, 0, 0, 0);
    let patterns: Vec<Value> = stats
        .iter()
        .map(|(slug, s)| {
            let trend = trend_word(s, now);
            match trend {
                "landing" => landing += 1,
                "worsening" => worsening += 1,
                "persisting" => persisting += 1,
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
        },
        "patterns": patterns,
    });
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}

/// The trend as a bare word (no glyph) for JSON consumers. Mirrors
/// [`trend_label`] so the two never diverge.
fn trend_word(s: &SlugStat, now: DateTime<Utc>) -> &'static str {
    match trend_label(s.last7, s.prior7, now - s.last) {
        "✓ dormant" => "dormant",
        "↑ worsening" => "worsening",
        "→ persisting" => "persisting",
        _ => "landing",
    }
}

/// Trend from recurrence: dormant if nothing in 14 days; otherwise compare the
/// last 7 days against the prior 7.
fn trend_label(last7: usize, prior7: usize, since_last: Duration) -> &'static str {
    if since_last > Duration::days(14) {
        "✓ dormant"
    } else if last7 == 0 {
        "↓ landing"
    } else if last7 < prior7 {
        "↓ landing"
    } else if last7 > prior7 {
        "↑ worsening"
    } else {
        "→ persisting"
    }
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

    #[test]
    fn trend_dormant_when_stale() {
        assert_eq!(trend_label(0, 0, Duration::days(20)), "✓ dormant");
    }

    #[test]
    fn trend_landing_when_quiet_recently() {
        assert_eq!(trend_label(0, 3, Duration::days(2)), "↓ landing");
        assert_eq!(trend_label(1, 4, Duration::days(1)), "↓ landing");
    }

    #[test]
    fn trend_worsening_when_more_recently() {
        assert_eq!(trend_label(4, 1, Duration::days(1)), "↑ worsening");
    }

    #[test]
    fn trend_persisting_when_flat() {
        assert_eq!(trend_label(2, 2, Duration::days(1)), "→ persisting");
    }

    #[test]
    fn sev_rank_orders() {
        assert!(sev_rank("S3") > sev_rank("S2"));
        assert_eq!(sev_rank("x"), 0);
    }
}
