//! Everything i-dream knows about, as one flat list a person can search.
//!
//! `i-dream index --json` gathers every pattern, association, finding,
//! evidence row, mistake type, proposal, input source, hint and nudge,
//! intention, scheduled job and session into rows of the same shape, so the
//! widget's search can find any of them by any word in it. Each row names the
//! ids it relates to, which is how a surface follows one item to the next.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// One searchable item.
#[derive(Debug, Serialize)]
pub struct Entry {
    /// pattern, association, finding, evidence, slug, proposal, source, hint,
    /// nudge, intention, job or session.
    pub kind: &'static str,
    pub id: String,
    /// The line a person recognises it by.
    pub title: String,
    /// Everything else worth reading about it, in plain words.
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    /// Ids of the items this one points at (pattern ids, evidence ids, a slug,
    /// a session, a cluster).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IndexDoc {
    pub generated: DateTime<Utc>,
    /// Rows per kind, so a surface can show what it searched.
    pub counts: BTreeMap<&'static str, usize>,
    pub entries: Vec<Entry>,
}

pub fn gather(home: &Path) -> IndexDoc {
    let mut entries = vec![];
    patterns(home, &mut entries);
    reader(home, &mut entries);
    slugs(&mut entries);
    proposals(home, &mut entries);
    sources(home, &mut entries);
    interventions(home, &mut entries);
    intentions(home, &mut entries);
    jobs(&mut entries);
    sessions(home, &mut entries);
    let mut counts = BTreeMap::new();
    for e in &entries {
        *counts.entry(e.kind).or_insert(0) += 1;
    }
    IndexDoc { generated: Utc::now(), counts, entries }
}

fn entry(kind: &'static str, id: impl Into<String>, title: impl Into<String>, detail: impl Into<String>) -> Entry {
    Entry { kind, id: id.into(), title: title.into(), detail: detail.into(), at: None, related: vec![] }
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

fn jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn patterns(home: &Path, out: &mut Vec<Entry>) {
    let (rows, assocs) = crate::status::pattern_rows(home);
    for (rank, p) in rows.iter().enumerate() {
        let mut e = entry(
            "pattern",
            &p.id,
            &p.text,
            format!(
                "{} · {} · strength {:.2}, rank {} of {} · seen {} times, {} in the last 7 days",
                p.category,
                p.valence,
                p.strength,
                rank + 1,
                rows.len(),
                p.occurrences,
                p.last7
            ),
        );
        e.at = parse_ts(&p.last_seen);
        out.push(e);
    }
    let text: BTreeMap<&str, &str> = rows.iter().map(|p| (p.id.as_str(), p.text.as_str())).collect();
    for a in &assocs {
        let mut e = entry(
            "association",
            &a.id,
            &a.hypothesis,
            format!(
                "links \"{}\" and \"{}\" · confidence {:.2}",
                text.get(a.a.as_str()).unwrap_or(&"?"),
                text.get(a.b.as_str()).unwrap_or(&"?"),
                a.confidence
            ),
        );
        e.related = vec![a.a.clone(), a.b.clone()];
        out.push(e);
    }
}

fn reader(home: &Path, out: &mut Vec<Entry>) {
    for run in crate::reader::all_runs() {
        for n in &run.items {
            let mut e = entry(
                "finding",
                &n.cluster,
                &n.title,
                format!("{} · weekly review {} · {}", n.kind, run.week, n.why_repeats),
            );
            e.at = run.at;
            e.related = n.evidence_ids.clone();
            out.push(e);
        }
    }
    // Every evidence row the latest recon read, not only the ones its
    // clusters cite.
    let Some(recon) = crate::reader::latest_recon() else { return };
    let day = recon.at.format("%Y-%m-%d").to_string();
    let path = home.join(".claude/i-dream/reader/daily").join(format!("{day}.evidence.jsonl"));
    for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
        let Ok(ev) = serde_json::from_str::<crate::reader::evidence::Evidence>(line) else { continue };
        let mut e = entry(
            "evidence",
            &ev.id,
            &ev.text,
            format!(
                "{} · by {}{}{}",
                ev.domain,
                ev.provenance,
                ev.project.as_deref().map(|p| format!(" · {p}")).unwrap_or_default(),
                ev.slug.as_deref().map(|s| format!(" · {s}")).unwrap_or_default()
            ),
        );
        e.at = Some(ev.ts);
        e.related = ev.slug.iter().chain(ev.session.iter()).cloned().collect();
        out.push(e);
    }
}

fn slugs(out: &mut Vec<Entry>) {
    for s in crate::reflect::slug_trends(Utc::now()) {
        let mut e = entry(
            "slug",
            &s.slug,
            &s.slug,
            format!(
                "{} · {} recorded in all · {} in the last 7 days, {} the 7 before",
                s.severity, s.total, s.last7, s.prior7
            ),
        );
        e.at = Some(s.last);
        out.push(e);
    }
}

fn proposals(home: &Path, out: &mut Vec<Entry>) {
    for p in jsonl(&home.join(".claude/proposals.jsonl")) {
        let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if s("id").is_empty() {
            continue;
        }
        let body: String = s("body").chars().take(400).collect();
        let mut e = entry(
            "proposal",
            s("id"),
            s("title"),
            format!("{} · {} · {}", s("status"), s("category"), body),
        );
        e.at = parse_ts(&s("ts"));
        out.push(e);
    }
}

fn sources(home: &Path, out: &mut Vec<Entry>) {
    for l in crate::modules::registry::compute_lane_health(home) {
        out.push(entry(
            "source",
            l.lane,
            l.lane,
            format!("{} · read by {} ({})", l.reason, l.consumer, l.consumer_state),
        ));
    }
}

fn interventions(home: &Path, out: &mut Vec<Entry>) {
    let items =
        crate::interventions::load_interventions(&home.join(".claude/i-dream/interventions.json")).unwrap_or_default();
    for i in items {
        let kind = if i.form == "nudge" { "nudge" } else { "hint" };
        let how = match (i.promoted, i.promoted_by.as_deref()) {
            (Some(t), Some("owner-flip")) => format!(" · you made it live {}", t.format("%Y-%m-%d")),
            (Some(t), _) => format!(" · went live on its evidence {}", t.format("%Y-%m-%d")),
            _ => String::new(),
        };
        let mut e = entry(kind, &i.id, &i.body, format!("{} · about {}{}", i.state, i.slug, how));
        e.at = i.promoted.or(Some(i.created));
        e.related = vec![i.slug.clone()];
        out.push(e);
    }
}

fn intentions(home: &Path, out: &mut Vec<Entry>) {
    for i in jsonl(&home.join(".claude/subconscious/intentions/registry.jsonl")) {
        let id = i.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let msg = i.pointer("/action/message").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        if id.is_empty() {
            continue;
        }
        let keywords: Vec<&str> = i
            .pointer("/trigger/keywords")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|k| k.as_str()).collect())
            .unwrap_or_default();
        let fired = i.get("fire_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let max = i.get("max_fires").and_then(|v| v.as_u64()).unwrap_or(0);
        let source = i.pointer("/action/source").and_then(|v| v.as_str()).unwrap_or_default();
        let title = if msg.is_empty() { format!("intention with no message ({})", keywords.join(", ")) } else { msg };
        let mut e = entry(
            "intention",
            id,
            title,
            format!("keywords {} · fired {fired} of {max} · from {source}", keywords.join(", ")),
        );
        e.at = i.get("created").and_then(|v| v.as_str()).and_then(parse_ts);
        out.push(e);
    }
}

fn jobs(out: &mut Vec<Entry>) {
    for j in crate::status::gather_jobs() {
        out.push(entry("job", &j.name, &j.desc, format!("{} · runs {}", j.schedule, j.command)));
    }
}

/// Sessions that i-dream showed something to, from the firing ledger: which
/// items each one was shown, and when.
fn sessions(home: &Path, out: &mut Vec<Entry>) {
    let mut by: BTreeMap<String, (std::collections::BTreeSet<String>, i64, i64)> = BTreeMap::new();
    for r in jsonl(&home.join(".claude/i-dream/would-fire.jsonl")) {
        let (Some(sid), Some(id), Some(ts)) =
            (r.get("sid").and_then(|v| v.as_str()), r.get("id").and_then(|v| v.as_str()), r.get("ts").and_then(|v| v.as_i64()))
        else {
            continue;
        };
        if r.get("state").and_then(|v| v.as_str()) != Some("live") {
            continue;
        }
        let s = by.entry(sid.to_string()).or_insert((Default::default(), i64::MAX, 0));
        s.0.insert(id.to_string());
        s.1 = s.1.min(ts);
        s.2 = s.2.max(ts);
    }
    for (sid, (items, first, last)) in by {
        let when = |t: i64| DateTime::from_timestamp(t, 0).map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
        let mut e = entry(
            "session",
            &sid,
            format!("session {}", sid.chars().take(8).collect::<String>()),
            format!("shown {} live item(s) between {} and {} UTC", items.len(), when(first), when(last)),
        );
        e.at = DateTime::from_timestamp(last, 0);
        e.related = items.into_iter().collect();
        out.push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_rows_are_skipped_and_never_panic() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        std::fs::create_dir_all(h.join(".claude/i-dream")).unwrap();
        std::fs::create_dir_all(h.join(".claude/subconscious/intentions")).unwrap();
        std::fs::write(
            h.join(".claude/i-dream/would-fire.jsonl"),
            "{\"id\":\"a\",\"sid\":\"ü\",\"state\":\"live\",\"ts\":1}\n\
             {\"id\":\"b\",\"sid\":\"aéééééééé-long\",\"state\":\"live\",\"ts\":2}\n\
             {\"id\":\"c\",\"sid\":\"x\",\"state\":\"shadow\",\"ts\":3}\nnot json\n",
        )
        .unwrap();
        std::fs::write(h.join(".claude/proposals.jsonl"), "{\"title\":\"no id\"}\n{\"id\":\"p1\",\"title\":\"t\"}\n").unwrap();
        std::fs::write(h.join(".claude/subconscious/intentions/registry.jsonl"), "{\"id\":\"i1\"}\n{}\n").unwrap();
        let mut out = vec![];
        sessions(h, &mut out);
        proposals(h, &mut out);
        intentions(h, &mut out);
        let ids: Vec<(&str, &str)> = out.iter().map(|e| (e.kind, e.id.as_str())).collect();
        // Shadow rows are not "shown"; a proposal or intention without an id is skipped.
        assert_eq!(
            ids,
            vec![("session", "aéééééééé-long"), ("session", "ü"), ("proposal", "p1"), ("intention", "i1")]
        );
        assert_eq!(out[0].title, "session aééééééé");
    }
}
