//! L2 daily digest — deterministic phase.
//!
//! Builds a fixed-shape markdown file at `~/.claude/i-dream/daily/YYYY-MM-DD.md`
//! that always has the same 7 section headings (per `docs/16-consolidation-build.md`
//! §3.4). Sections that need LLM enrichment (Top signals, Cross-domain
//! associations) ship with a placeholder until Stage 3 wires the dream pass.
//!
//! Renders + writes are idempotent — running twice on the same date overwrites
//! with the same content.

use crate::config::Config;
use crate::modules::registry::DomainRegistry;
use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, Utc};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

// ── Day bundle: the deterministic inputs collected for one day ─────────────

#[derive(Debug, Default)]
pub struct DayBundle {
    pub date: NaiveDate,
    /// One entry per registered domain. Sorted by name for stable render order.
    pub per_domain: BTreeMap<String, DomainSlice>,
    pub sources: Vec<SourceLink>,
}

#[derive(Debug, Default)]
pub struct DomainSlice {
    pub raw_event_count: usize,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum SourceKind {
    CogitateTopic,
    SkillReport,
    DreamCycle,
    Rca,
    Other,
}

impl SourceKind {
    fn label(&self) -> &'static str {
        match self {
            Self::CogitateTopic => "topic",
            Self::SkillReport => "report",
            Self::DreamCycle => "dream",
            Self::Rca => "rca",
            Self::Other => "other",
        }
    }
}

#[derive(Debug)]
pub struct SourceLink {
    pub path: PathBuf,
    pub kind: SourceKind,
    pub title: Option<String>,
    pub modified: SystemTime,
}

// ── Gather: build a DayBundle for the given date ───────────────────────────

pub fn gather_day_bundle(date: NaiveDate, config: &Config, store: &Store) -> Result<DayBundle> {
    let registry = DomainRegistry::boot(config, store);
    let mut bundle = DayBundle {
        date,
        ..Default::default()
    };
    for d in registry.iter() {
        bundle
            .per_domain
            .insert(d.name().to_string(), DomainSlice::default());
    }
    bundle.sources = scan_one_off_sources(date)?;
    Ok(bundle)
}

/// Walk known one-off-report dirs and collect files modified on the given
/// local date. Bounded — top-level scan only, no recursion past one level.
fn scan_one_off_sources(date: NaiveDate) -> Result<Vec<SourceLink>> {
    let home = std::env::var("HOME").context("HOME unset")?;
    let probes: Vec<(PathBuf, SourceKind)> = vec![
        (
            PathBuf::from(&home).join(".claude/topics"),
            SourceKind::CogitateTopic,
        ),
        (
            PathBuf::from(&home).join(".claude/assets/reports"),
            SourceKind::SkillReport,
        ),
        (
            PathBuf::from(&home).join(".claude/subconscious/dreams"),
            SourceKind::DreamCycle,
        ),
    ];
    let mut sources = vec![];
    for (root, kind) in probes {
        if !root.exists() {
            continue;
        }
        let entries = match fs::read_dir(&root) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let modified = match meta.modified() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !was_modified_on(modified, date) {
                continue;
            }
            let title = extract_title(&path);
            sources.push(SourceLink {
                path,
                kind,
                title,
                modified,
            });
        }
    }
    // Stable order: newest first
    sources.sort_by_key(|s| std::cmp::Reverse(s.modified));
    Ok(sources)
}

fn was_modified_on(mtime: SystemTime, date: NaiveDate) -> bool {
    let dt: DateTime<Utc> = mtime.into();
    let local = dt.with_timezone(&Local);
    local.date_naive() == date
}

fn extract_title(path: &Path) -> Option<String> {
    // For directories, use the dir name.
    if path.is_dir() {
        return path.file_name().and_then(|s| s.to_str()).map(String::from);
    }
    // For markdown files, read first H1.
    if path.extension().and_then(|e| e.to_str()) == Some("md")
        && let Ok(content) = fs::read_to_string(path)
    {
        for line in content.lines().take(40) {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("# ") {
                return Some(rest.trim().to_string());
            }
        }
    }
    // Fallback: file stem.
    path.file_stem().and_then(|s| s.to_str()).map(String::from)
}

// ── Render: turn a DayBundle into the 7-section markdown ───────────────────

pub fn render_markdown(bundle: &DayBundle) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(&format!("# {} — i-dream daily\n\n", bundle.date));

    out.push_str("## Top signals\n\n");
    let tldr_lines = read_tldr_union();
    if tldr_lines.is_empty() {
        out.push_str("_(no signals yet — run `i-dream dream-pass` to populate)_\n\n");
    } else {
        for line in &tldr_lines {
            out.push_str(&format!("{line}\n"));
        }
        out.push('\n');
    }

    out.push_str("## Per-domain summary\n\n");
    if bundle.per_domain.is_empty() {
        out.push_str("_(no registered domains)_\n\n");
    } else {
        for (name, slice) in &bundle.per_domain {
            out.push_str(&format!("### {name}\n"));
            if slice.raw_event_count == 0 && slice.note.is_none() {
                out.push_str("_(no activity tracked yet)_\n\n");
            } else {
                if slice.raw_event_count > 0 {
                    out.push_str(&format!("- {} new events\n", slice.raw_event_count));
                }
                if let Some(n) = &slice.note {
                    out.push_str(&format!("- {n}\n"));
                }
                out.push('\n');
            }
        }
    }

    out.push_str("## Pinned from sessions\n\n");
    let pinned_md = read_pinned_active();
    if pinned_md.trim().is_empty() {
        out.push_str("_(no active pins — use `/pin-for-dream` or `i-dream pin add`)_\n\n");
    } else {
        out.push_str(&pinned_md);
        if !pinned_md.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("## Cross-domain associations\n\n");
    let associations = read_cross_associations(bundle.date);
    if associations.is_empty() {
        out.push_str("_(none today — run `i-dream dream-pass` to populate)_\n\n");
    } else {
        for assoc in &associations {
            out.push_str(&format!("- {assoc}\n"));
        }
        out.push('\n');
    }

    out.push_str("## Open threads (carried over)\n\n");
    match crate::thread::open_threads() {
        Ok(threads) if !threads.is_empty() => {
            for t in &threads {
                let age = (chrono::Utc::now() - t.opened).num_days();
                out.push_str(&format!("- `{}` ({age}d) — {}\n", t.id, t.text));
            }
            out.push('\n');
        }
        _ => out.push_str("_(no open threads)_\n\n"),
    }

    out.push_str("## Sources\n\n");
    if bundle.sources.is_empty() {
        out.push_str("_(no one-off reports landed today)_\n\n");
    } else {
        for src in &bundle.sources {
            let title = src.title.as_deref().unwrap_or("(untitled)");
            let path_str = src.path.display();
            out.push_str(&format!(
                "- [{title}]({path_str}) _({})_\n",
                src.kind.label()
            ));
        }
        out.push('\n');
    }

    out.push_str("## Queued for Sunday audit\n\n");
    out.push_str("- Graduation candidates: 0\n");
    out.push_str("- Stale threads: 0\n");
    out.push_str("- Pending GCC proposals: 0\n\n");

    out.push_str("---\n\n");
    out.push_str("_Rendered by i-dream/l2-digest (Stage 2 deterministic)._\n");
    out
}

// ── Write: place the file on disk + update `latest.md` symlink ─────────────

/// Build, render, and write the daily file for `date`. Returns the absolute
/// path written. Idempotent — overwrites existing same-date file.
pub fn write_daily(date: NaiveDate, config: &Config, store: &Store) -> Result<PathBuf> {
    let bundle = gather_day_bundle(date, config, store)?;
    let markdown = render_markdown(&bundle);

    let daily_dir = daily_dir()?;
    fs::create_dir_all(&daily_dir)
        .with_context(|| format!("Cannot create {}", daily_dir.display()))?;

    let path = daily_dir.join(format!("{date}.md"));
    let tmp = daily_dir.join(format!(".{date}.md.tmp"));
    fs::write(&tmp, &markdown).with_context(|| format!("Cannot write {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("Cannot rename {} -> {}", tmp.display(), path.display()))?;

    // Update `latest.md` symlink — point at today if `date` equals today,
    // otherwise leave the symlink alone (we don't want a `--day` flag of a
    // past date to silently retarget the user's "today" view).
    if date == Local::now().naive_local().date() {
        update_latest_symlink(&daily_dir, &path)?;
    }

    Ok(path)
}

fn daily_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".claude/i-dream/daily"))
}

/// Read `~/.claude/pinned/derived/active.md` — pre-rendered by the pinned
/// domain's consolidate.sh. Empty when no active pins exist (or no plugin).
fn read_pinned_active() -> String {
    let Ok(home) = std::env::var("HOME") else {
        return String::new();
    };
    let path = PathBuf::from(home).join(".claude/pinned/derived/active.md");
    fs::read_to_string(&path).unwrap_or_default()
}

/// Read `~/.claude/i-dream/derived/tldr.union.txt` produced by the last
/// `i-dream dream-pass`. Returns empty Vec if the file doesn't exist —
/// digest then falls back to a placeholder. Each line is rendered verbatim.
fn read_tldr_union() -> Vec<String> {
    let Ok(home) = std::env::var("HOME") else {
        return vec![];
    };
    let path = PathBuf::from(home).join(".claude/i-dream/derived/tldr.union.txt");
    let Ok(content) = fs::read_to_string(&path) else {
        return vec![];
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .map(|s| s.to_string())
        .collect()
}

/// Read cross-domain associations produced by the last DreamPass that ran
/// today (matched by file mtime). Returns one-liner strings ready to render
/// as digest bullets. Empty when the file is absent or stale.
fn read_cross_associations(date: NaiveDate) -> Vec<String> {
    let Ok(home) = std::env::var("HOME") else {
        return vec![];
    };
    let path = PathBuf::from(home).join(".claude/i-dream/derived/associations.cross.jsonl");
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return vec![],
    };
    let modified = match meta.modified() {
        Ok(m) => m,
        Err(_) => return vec![],
    };
    if !was_modified_on(modified, date) {
        return vec![];
    }
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let from_d = v.get("from_domain")?.as_str()?;
            let from_s = v.get("from_slug")?.as_str()?;
            let to_d = v.get("to_domain")?.as_str()?;
            let to_s = v.get("to_slug")?.as_str()?;
            let conf = v.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.0);
            let instruction = v
                .get("instruction")
                .and_then(|s| s.as_str())
                .unwrap_or("(no instruction)");
            Some(format!(
                "**{from_s}** ({from_d}) ↔ **{to_s}** ({to_d}) — {instruction} _(conf {conf:.2})_"
            ))
        })
        .take(5)
        .collect()
}

fn update_latest_symlink(daily_dir: &Path, target: &Path) -> Result<()> {
    let latest = daily_dir.join("latest.md");
    // Remove existing symlink/file (ignore errors if it didn't exist).
    let _ = fs::remove_file(&latest);
    // Symlink relative to daily_dir for portability.
    let rel = target
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| target.to_path_buf());
    std::os::unix::fs::symlink(&rel, &latest)
        .with_context(|| format!("Cannot symlink {} -> {}", latest.display(), rel.display()))?;
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn empty_bundle(date: NaiveDate) -> DayBundle {
        DayBundle {
            date,
            ..Default::default()
        }
    }

    #[test]
    fn render_includes_all_seven_section_headings_even_when_empty() {
        let bundle = empty_bundle(NaiveDate::from_ymd_opt(2026, 5, 16).unwrap());
        let md = render_markdown(&bundle);
        for heading in [
            "## Top signals",
            "## Per-domain summary",
            "## Pinned from sessions",
            "## Cross-domain associations",
            "## Open threads",
            "## Sources",
            "## Queued for Sunday audit",
        ] {
            assert!(
                md.contains(heading),
                "missing section heading: {heading}\nrender:\n{md}"
            );
        }
    }

    #[test]
    fn render_h1_carries_the_date() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 16).unwrap();
        let md = render_markdown(&empty_bundle(date));
        assert!(md.starts_with("# 2026-05-16 — i-dream daily"));
    }

    #[test]
    fn render_per_domain_shows_placeholder_for_empty_slices() {
        let mut bundle = empty_bundle(NaiveDate::from_ymd_opt(2026, 5, 16).unwrap());
        bundle
            .per_domain
            .insert("atone".into(), DomainSlice::default());
        bundle
            .per_domain
            .insert("dreaming".into(), DomainSlice::default());
        let md = render_markdown(&bundle);
        assert!(md.contains("### atone"));
        assert!(md.contains("### dreaming"));
        // Empty slices get the italic placeholder
        assert!(md.matches("_(no activity tracked yet)_").count() == 2);
    }

    #[test]
    fn render_per_domain_shows_event_count_when_present() {
        let mut bundle = empty_bundle(NaiveDate::from_ymd_opt(2026, 5, 16).unwrap());
        bundle.per_domain.insert(
            "atone".into(),
            DomainSlice {
                raw_event_count: 3,
                note: None,
            },
        );
        let md = render_markdown(&bundle);
        assert!(md.contains("3 new events"));
    }

    #[test]
    fn render_sources_section_lists_each_link() {
        let mut bundle = empty_bundle(NaiveDate::from_ymd_opt(2026, 5, 16).unwrap());
        bundle.sources.push(SourceLink {
            path: PathBuf::from("/tmp/topic-a.md"),
            kind: SourceKind::CogitateTopic,
            title: Some("Topic A".into()),
            modified: SystemTime::now(),
        });
        bundle.sources.push(SourceLink {
            path: PathBuf::from("/tmp/report-b"),
            kind: SourceKind::SkillReport,
            title: Some("Report B".into()),
            modified: SystemTime::now(),
        });
        let md = render_markdown(&bundle);
        assert!(md.contains("[Topic A](/tmp/topic-a.md) _(topic)_"));
        assert!(md.contains("[Report B](/tmp/report-b) _(report)_"));
    }

    #[test]
    fn was_modified_on_matches_same_local_day() {
        let now = SystemTime::now();
        let today = Local::now().naive_local().date();
        assert!(was_modified_on(now, today));
        let way_back = now - Duration::from_secs(60 * 60 * 24 * 7);
        assert!(!was_modified_on(way_back, today));
    }

    #[test]
    fn extract_title_reads_first_h1_from_markdown() {
        let dir = std::env::temp_dir().join(format!("idream-l2-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.md");
        fs::write(&path, "Some preface\n# Real Title\n\nbody").unwrap();
        assert_eq!(extract_title(&path).as_deref(), Some("Real Title"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_title_falls_back_to_file_stem_when_no_h1() {
        let dir = std::env::temp_dir().join(format!("idream-l2-fallback-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("no-h1-here.md");
        fs::write(&path, "no h1 in this file\njust some text").unwrap();
        assert_eq!(extract_title(&path).as_deref(), Some("no-h1-here"));
        let _ = fs::remove_dir_all(&dir);
    }
}
