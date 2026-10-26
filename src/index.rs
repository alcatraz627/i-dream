//! Everything i-dream knows about, as one flat list a person can search.
//!
//! `i-dream index --json` gathers every pattern, association, finding,
//! evidence row, mistake type, proposal, input source, hint and nudge,
//! intention, insight, armed gate, scheduled job and session into rows of the same shape, so the
//! widget's search can find any of them by any word in it. Each row names the
//! ids it relates to, which is how a surface follows one item to the next.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// One searchable item.
#[derive(Debug, Serialize)]
pub struct Entry {
    /// pattern, association, insight, finding, evidence, slug, proposal,
    /// source, hint, nudge, intention, gate, job or session.
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
    /// The columns a list of this kind shows, by name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<&'static str, String>,
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
    let firings = Firings::load(home);
    patterns(home, &firings, &mut entries);
    reader(home, &mut entries);
    slugs(&mut entries);
    proposals(home, &mut entries);
    sources(home, &mut entries);
    interventions(home, &firings, &mut entries);
    intentions(home, &firings, &mut entries);
    jobs(&mut entries);
    sessions(home, &firings, &mut entries);
    let mut counts = BTreeMap::new();
    for e in &entries {
        *counts.entry(e.kind).or_insert(0) += 1;
    }
    IndexDoc { generated: Utc::now(), counts, entries }
}

fn entry(kind: &'static str, id: impl Into<String>, title: impl Into<String>, detail: impl Into<String>) -> Entry {
    Entry { kind, id: id.into(), title: title.into(), detail: detail.into(), at: None, related: vec![], fields: BTreeMap::new() }
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

/// What the firing scan found for each item shown to a session: how many
/// sessions echoed its tag (cited) and how many showed it unused.
#[derive(Default)]
struct Firings {
    /// item key ("N:<hint id>", "I:<intention id>", or an insight id) to (cited, unused).
    by_item: BTreeMap<String, (usize, usize)>,
    /// session id to the item keys it cited.
    cited_in: BTreeMap<String, Vec<String>>,
}

impl Firings {
    fn load(home: &Path) -> Self {
        let mut f = Firings::default();
        for r in jsonl(&home.join(".claude/subconscious/dreams/firings.jsonl")) {
            let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or_default();
            let cited = kind == "fired";
            if !cited && kind != "present-unused" {
                continue;
            }
            let mut keys: Vec<String> = r.get("id").and_then(|v| v.as_str()).map(String::from).into_iter().collect();
            keys.extend(r.get("insight_id").and_then(|v| v.as_str()).map(String::from));
            for k in &keys {
                let e = f.by_item.entry(k.clone()).or_default();
                if cited { e.0 += 1 } else { e.1 += 1 }
            }
            if cited {
                if let (Some(sid), Some(id)) = (r.get("sid").and_then(|v| v.as_str()), keys.first()) {
                    f.cited_in.entry(sid.to_string()).or_default().push(id.clone());
                }
            }
        }
        f
    }

    /// "cited in 2 of 5 sessions checked", or "not checked yet" before any scan judged it.
    fn cited(&self, key: &str) -> String {
        match self.by_item.get(key) {
            Some(&(c, u)) => format!("cited in {c} of {} sessions checked", c + u),
            None => "not checked yet".to_string(),
        }
    }
}

fn patterns(home: &Path, firings: &Firings, out: &mut Vec<Entry>) {
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
    // An insight is an association the wake phase promoted; the raw store
    // says which, and which intention each one produced.
    let raw: Vec<crate::modules::dreaming::Association> =
        std::fs::read_to_string(home.join(".claude/subconscious/dreams/associations.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    let raw: BTreeMap<&str, &crate::modules::dreaming::Association> = raw.iter().map(|a| (a.id.as_str(), a)).collect();
    for a in &assocs {
        let full = raw.get(a.id.as_str());
        let insight = full.is_some_and(|f| f.promoted);
        let mut e = entry(
            if insight { "insight" } else { "association" },
            &a.id,
            &a.hypothesis,
            format!(
                "links \"{}\" and \"{}\" · confidence {:.2}{}",
                text.get(a.a.as_str()).unwrap_or(&"?"),
                text.get(a.b.as_str()).unwrap_or(&"?"),
                a.confidence,
                full.and_then(|f| f.suggested_rule.as_deref()).map(|r| format!(" · rule: {r}")).unwrap_or_default()
            ),
        );
        e.related = vec![a.a.clone(), a.b.clone()];
        e.related.extend(full.and_then(|f| f.auto_intention_id.clone()));
        e.fields.insert("confidence", format!("{:.2}", a.confidence));
        if insight {
            e.fields.insert("cited", firings.cited(&a.id));
            e.fields.insert(
                "intention",
                full.and_then(|f| f.auto_intention_id.clone()).unwrap_or_else(|| "none".into()),
            );
        }
        out.push(e);
    }
}

fn reader(home: &Path, out: &mut Vec<Entry>) {
    for run in crate::reader::all_runs() {
        // An armed answer switched a hook on: that gate's home is this list.
        for l in run.landed.iter().filter(|l| l.outcome == "armed") {
            let mut e = entry(
                "gate",
                format!("gate:{}", l.cluster),
                &l.title,
                format!("armed from your answer to weekly review {} · switch: {}", run.week, l.detail.as_deref().unwrap_or("?")),
            );
            e.at = Some(l.ts);
            e.related = vec![l.cluster.clone()];
            e.fields.insert("week", run.week.clone());
            out.push(e);
        }
        // Every named finding, as the Reader pane shows them: noise is named
        // too but is not a finding, and some never reach the page.
        for n in run.named.items.iter().filter(|n| n.kind != "noise") {
            let on_page = run.items.iter().any(|i| i.cluster == n.cluster);
            let mut e = entry(
                "finding",
                &n.cluster,
                &n.title,
                format!(
                    "{} · weekly review {}{} · {}",
                    n.kind,
                    run.week,
                    if on_page { "" } else { " · not put on the page" },
                    if n.why_repeats.is_empty() { &n.why } else { &n.why_repeats }
                ),
            );
            e.at = run.at;
            e.related = n.evidence_ids.clone();
            // The proposal this finding was filed as, when it was.
            e.related.extend(
                run.landed
                    .iter()
                    .filter(|l| l.cluster == n.cluster && l.outcome.contains("filed"))
                    .filter_map(|l| l.detail.clone()),
            );
            e.fields.insert("kind", n.kind.clone());
            e.fields.insert("week", run.week.clone());
            out.push(e);
        }
    }
    // Every evidence row any kept daily recon read, newest copy of each, so
    // an older finding's citations resolve too.
    let mut rows: BTreeMap<String, crate::reader::evidence::Evidence> = BTreeMap::new();
    let dir = home.join(".claude/i-dream/reader/daily");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|f| f.path()).filter(|p| p.to_string_lossy().ends_with(".evidence.jsonl")).collect())
        .unwrap_or_default();
    files.sort();
    for f in files {
        for line in std::fs::read_to_string(f).unwrap_or_default().lines() {
            if let Ok(ev) = serde_json::from_str::<crate::reader::evidence::Evidence>(line) {
                rows.insert(ev.id.clone(), ev);
            }
        }
    }
    for ev in rows.values() {
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
        e.fields.insert("domain", ev.domain.clone());
        e.fields.insert("by", ev.provenance.clone());
        out.push(e);
    }
    // Atone events older than every kept daily file are still cited by older
    // findings; the atone log itself is their source.
    for a in jsonl(&home.join(".claude/atone/events.jsonl")) {
        let s = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let id = format!("atone:{}", s("id"));
        if s("id").is_empty() || rows.contains_key(&id) {
            continue;
        }
        let mut e = entry("evidence", &id, s("title"), format!("atone · {} · {} · {}", s("severity"), s("slug"), s("issue")));
        e.at = parse_ts(&s("ts"));
        e.related = [s("slug"), s("session_id")].into_iter().filter(|x| !x.is_empty()).collect();
        e.fields.insert("domain", "atone".into());
        e.fields.insert("by", "atone log".into());
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
        e.fields.insert("severity", s.severity.clone());
        e.fields.insert("total", s.total.to_string());
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
        e.fields.insert("status", s("status"));
        e.fields.insert("category", s("category"));
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

fn interventions(home: &Path, firings: &Firings, out: &mut Vec<Entry>) {
    let mut shown: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for r in jsonl(&home.join(".claude/i-dream/would-fire.jsonl")) {
        if let (Some(id), Some(sid)) = (r.get("id").and_then(|v| v.as_str()), r.get("sid").and_then(|v| v.as_str())) {
            shown.entry(id.to_string()).or_default().insert(sid.to_string());
        }
    }
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
        e.fields.insert("state", i.state.clone());
        e.fields.insert("sessions", shown.get(&i.id).map_or(0, |s| s.len()).to_string());
        e.fields.insert("cited", firings.cited(&format!("N:{}", i.id)));
        e.fields.insert(
            "promoted by",
            match i.promoted_by.as_deref() {
                Some("owner-flip") => "you".into(),
                Some("evidence-auto") => "its evidence".into(),
                Some(o) => o.into(),
                None => "not promoted".into(),
            },
        );
        out.push(e);
    }
}

fn intentions(home: &Path, firings: &Firings, out: &mut Vec<Entry>) {
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
        let cited = firings.cited(&format!("I:{id}"));
        let title = if msg.is_empty() { format!("intention with no message ({})", keywords.join(", ")) } else { msg };
        let mut e = entry(
            "intention",
            &id,
            title,
            format!("keywords {} · fired {fired} of {max} · from {source}", keywords.join(", ")),
        );
        e.at = i.get("created").and_then(|v| v.as_str()).and_then(parse_ts);
        e.fields.insert("fires", format!("{fired} of {max}"));
        e.fields.insert("cited", cited);
        e.fields.insert("keywords", keywords.join(", "));
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
fn sessions(home: &Path, firings: &Firings, out: &mut Vec<Entry>) {
    let transcripts = transcript_paths(home);
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
        let cited = firings.cited_in.get(&sid).map_or(0, |c| c.len());
        e.fields.insert("shown", items.len().to_string());
        e.fields.insert("cited", cited.to_string());
        if let Some(t) = transcripts.get(&sid) {
            e.fields.insert("transcript", t.clone());
        }
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
        let f = Firings::default();
        sessions(h, &f, &mut out);
        proposals(h, &mut out);
        intentions(h, &f, &mut out);
        let ids: Vec<(&str, &str)> = out.iter().map(|e| (e.kind, e.id.as_str())).collect();
        // Shadow rows are not "shown"; a proposal or intention without an id is skipped.
        assert_eq!(
            ids,
            vec![("session", "aéééééééé-long"), ("session", "ü"), ("proposal", "p1"), ("intention", "i1")]
        );
        assert_eq!(out[0].title, "session aééééééé");
    }
}

/// Where each session's transcript is, by session id, from one listing of
/// ~/.claude/projects rather than a search per session.
fn transcript_paths(home: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(projects) = std::fs::read_dir(home.join(".claude/projects")) else { return out };
    for p in projects.flatten() {
        let Ok(files) = std::fs::read_dir(p.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().is_some_and(|x| x == "jsonl") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    out.insert(stem.to_string(), path.display().to_string());
                }
            }
        }
    }
    out
}
