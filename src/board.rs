//! `i-dream board` — a one-screen snapshot of the dreaming layer: Today,
//! Week, Sources, and GCC-fitness in a 2×2 bordered grid. Static (renders the
//! current state and exits); re-run to refresh. Reads the same artifacts the
//! daily digest + audit produce, so it never does its own LLM work.

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use std::fs;
use std::path::PathBuf;

const PANE_W: usize = 38;
const PANE_H: usize = 9;

pub fn render() -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME unset")?);
    let daily_dir = home.join(".claude/i-dream/daily");
    let audits_dir = home.join(".claude/i-dream/audits");

    let latest = fs::read_to_string(daily_dir.join("latest.md")).unwrap_or_default();

    let today = pane("Today", &today_lines(&latest), PANE_W, PANE_H);
    let week = pane("Week", &week_lines(&daily_dir, &audits_dir), PANE_W, PANE_H);
    let sources = pane("Sources", &section(&latest, "Sources"), PANE_W, PANE_H);
    let fitness = pane("GCC fitness", &fitness_lines(&audits_dir), PANE_W, PANE_H);

    println!("  i-dream board · {}", Local::now().format("%Y-%m-%d %H:%M"));
    println!();
    for line in beside(&today, &week) {
        println!("  {line}");
    }
    for line in beside(&sources, &fitness) {
        println!("  {line}");
    }
    println!();
    println!("  (snapshot — re-run to refresh · `i-dream digest` to rebuild today)");
    Ok(())
}

fn today_lines(latest: &str) -> Vec<String> {
    if latest.trim().is_empty() {
        return vec!["(no digest yet — run".into(), "`i-dream digest`)".into()];
    }
    let mut out = vec!["▸ Top signals".to_string()];
    let signals = section(latest, "Top signals");
    out.extend(signals.into_iter().take(3));
    out.push("▸ Per-domain".to_string());
    // The per-domain section uses ### subsection headings per domain.
    for line in latest.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            out.push(format!("  · {name}"));
        }
    }
    out
}

fn week_lines(daily_dir: &PathBuf, audits_dir: &PathBuf) -> Vec<String> {
    let today = Local::now().date_naive();
    let dailies = count_recent_md(daily_dir, today, 7);
    let last_audit = latest_md_date(audits_dir);
    let mut out = vec![format!("Daily digests (7d): {dailies}")];
    match last_audit {
        Some(d) => {
            let age = (today - d).num_days();
            out.push(format!("Last audit: {d} ({age}d)"));
            out.push(if age >= 7 {
                "⚠ audit due (run weekly)".to_string()
            } else {
                "audit current".to_string()
            });
        }
        None => out.push("No audit run yet".to_string()),
    }
    out.push("".to_string());
    out.push("apply:  i-dream audit run".to_string());
    out
}

fn fitness_lines(audits_dir: &PathBuf) -> Vec<String> {
    let Some(d) = latest_md_date(audits_dir) else {
        return vec!["(no audit yet)".into(), "i-dream audit run".into()];
    };
    let path = audits_dir.join(format!("{d}.md"));
    let content = fs::read_to_string(&path).unwrap_or_default();
    let mut out = vec![format!("Latest: {d}")];
    // The audit log's second line is "Surfaced: N · Approved: N · Applied: N".
    if let Some(counts) = content
        .lines()
        .find(|l| l.starts_with("Surfaced:"))
    {
        out.push(counts.to_string());
    }
    // List the proposal targets (## Proposal N/M — agent _status_ lines carry
    // the target on the next "- Target:" line).
    let targets: Vec<String> = content
        .lines()
        .filter_map(|l| l.strip_prefix("- Target:"))
        .map(|t| format!("·{}", t.trim().trim_matches('`')))
        .collect();
    if targets.is_empty() {
        out.push("(no proposals)".to_string());
    } else {
        out.extend(targets.into_iter().take(4));
    }
    out
}

// ── section extraction ───────────────────────────────────────────────────────

/// Lines under a `## <heading>` until the next `## `, trimmed of blanks and the
/// "_(no …)_" placeholders the digest emits for empty sections.
fn section(md: &str, heading: &str) -> Vec<String> {
    let mut out = vec![];
    let mut in_section = false;
    for line in md.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            in_section = h.trim_start().starts_with(heading);
            continue;
        }
        if in_section {
            let t = line.trim();
            if t.is_empty() || (t.starts_with("_(") && t.ends_with(")_")) {
                continue;
            }
            out.push(t.trim_start_matches("- ").to_string());
        }
    }
    out
}

// ── filesystem helpers ─────────────────────────────────────────────────────────

/// Count `YYYY-MM-DD.md` files within `days` of `today` (inclusive).
fn count_recent_md(dir: &PathBuf, today: NaiveDate, days: i64) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| md_date(&e.path()))
        .filter(|d| {
            let age = (today - *d).num_days();
            (0..days).contains(&age)
        })
        .count()
}

/// Most recent `YYYY-MM-DD.md` date in a dir, ignoring symlinks like latest.md.
fn latest_md_date(dir: &PathBuf) -> Option<NaiveDate> {
    let entries = fs::read_dir(dir).ok()?;
    entries
        .flatten()
        .filter_map(|e| md_date(&e.path()))
        .max()
}

/// Parse a `YYYY-MM-DD.md` filename into a date. None for other names.
fn md_date(path: &std::path::Path) -> Option<NaiveDate> {
    let stem = path.file_stem()?.to_str()?;
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

// ── box rendering ──────────────────────────────────────────────────────────────

/// Fit a string to exactly `n` display chars: truncate with `…` or right-pad.
fn fit(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > n {
        let mut t: String = chars[..n.saturating_sub(1)].iter().collect();
        t.push('…');
        t
    } else {
        let mut t = s.to_string();
        t.push_str(&" ".repeat(n - chars.len()));
        t
    }
}

/// Render one titled box of fixed width + height (content padded/truncated).
fn pane(title: &str, lines: &[String], width: usize, height: usize) -> Vec<String> {
    let inner = width - 2;
    let mut out = Vec::new();
    let title_seg = format!("─ {title} ");
    let fill = inner.saturating_sub(title_seg.chars().count());
    out.push(format!("┌{}{}┐", title_seg, "─".repeat(fill)));
    for i in 0..height {
        let content = lines.get(i).map(String::as_str).unwrap_or("");
        out.push(format!("│{}│", fit(&format!(" {content}"), inner)));
    }
    out.push(format!("└{}┘", "─".repeat(inner)));
    out
}

/// Place two equal-height boxes side by side with a 2-space gutter.
fn beside(left: &[String], right: &[String]) -> Vec<String> {
    let h = left.len().max(right.len());
    (0..h)
        .map(|i| {
            let l = left.get(i).cloned().unwrap_or_default();
            let r = right.get(i).cloned().unwrap_or_default();
            format!("{l}  {r}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_extracts_and_skips_placeholder() {
        let md = "## Top signals\n\n- one\n- two\n\n## Sources\n\n_(no one-off reports)_\n";
        assert_eq!(section(md, "Top signals"), vec!["one", "two"]);
        assert!(section(md, "Sources").is_empty()); // placeholder skipped
    }

    #[test]
    fn pane_has_fixed_dimensions() {
        let p = pane("T", &["a".to_string()], 20, 4);
        assert_eq!(p.len(), 6); // top + 4 content + bottom
        assert!(p.iter().all(|l| l.chars().count() == 20));
    }

    #[test]
    fn beside_joins_rows() {
        let l = vec!["AAA".to_string(), "BBB".to_string()];
        let r = vec!["111".to_string(), "222".to_string()];
        assert_eq!(beside(&l, &r), vec!["AAA  111", "BBB  222"]);
    }

    #[test]
    fn md_date_parses_only_dated_files() {
        assert!(md_date(std::path::Path::new("/x/2026-05-22.md")).is_some());
        assert!(md_date(std::path::Path::new("/x/latest.md")).is_none());
    }
}
