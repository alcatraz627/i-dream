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
    /// source, hint, nudge, intention, gate, job, run, cycle or session.
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
    let wake = wake_intentions(home);
    patterns(home, &firings, &wake, &mut entries);
    reader(home, &mut entries);
    slugs(&mut entries);
    proposals(home, &filed_from(), &mut entries);
    sources(home, &mut entries);
    interventions(home, &firings, &mut entries);
    intentions(home, &firings, &mut entries);
    let jobs_now = crate::status::gather_jobs();
    runs(home, &jobs_now, &mut entries);
    jobs(&jobs_now, &mut entries);
    let transcripts = transcript_paths(home);
    sessions_at(home, &firings, &transcripts, &mut entries);
    fill_from_evidence(&firings, &transcripts, &mut entries);
    add_feedback(home, &firings, &transcripts, &mut entries);
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
    /// session id to (items it cited, items it was shown unused), for sessions a scan judged.
    by_session: BTreeMap<String, (usize, usize)>,
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
            if let Some(sid) = r.get("sid").and_then(|v| v.as_str()) {
                let e = f.by_session.entry(sid.to_string()).or_default();
                if cited { e.0 += 1 } else { e.1 += 1 }
            }
        }
        f
    }

    /// For a session: "cited 1 of 4 items shown", or "not checked yet".
    fn session_cited(&self, sid: &str) -> String {
        match self.by_session.get(sid) {
            Some(&(c, u)) => format!("cited {c} of {} items shown", c + u),
            None => "not checked yet".to_string(),
        }
    }

    /// "cited in 2 of 5 sessions checked", or "not checked yet" before any scan judged it.
    fn cited(&self, key: &str) -> String {
        match self.by_item.get(key) {
            Some(&(c, u)) => format!("cited in {c} of {} sessions checked", c + u),
            None => "not checked yet".to_string(),
        }
    }
}

/// Association id to the intention the wake phase made from it, read from
/// each intention's `dream-wake:<association>` source.
fn wake_intentions(home: &Path) -> BTreeMap<String, String> {
    jsonl(&home.join(".claude/subconscious/intentions/registry.jsonl"))
        .iter()
        .filter_map(|i| {
            let id = i.get("id")?.as_str()?;
            let src = i.pointer("/action/source")?.as_str()?;
            Some((src.strip_prefix("dream-wake:")?.to_string(), id.to_string()))
        })
        .collect()
}

fn patterns(home: &Path, firings: &Firings, wake: &BTreeMap<String, String>, out: &mut Vec<Entry>) {
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
        let made = full.and_then(|f| f.auto_intention_id.clone()).or_else(|| wake.get(&a.id).cloned());
        e.related = vec![a.a.clone(), a.b.clone()];
        e.related.extend(made.clone());
        e.fields.insert("confidence", format!("{:.2}", a.confidence));
        if insight {
            e.fields.insert("cited", firings.cited(&a.id));
            e.fields.insert("intention", made.unwrap_or_else(|| "none made".into()));
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
    let mut source_of: BTreeMap<String, String> = BTreeMap::new();
    let dir = home.join(".claude/i-dream/reader/daily");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|f| f.path()).filter(|p| p.to_string_lossy().ends_with(".evidence.jsonl")).collect())
        .unwrap_or_default();
    files.sort();
    for f in files {
        for line in std::fs::read_to_string(&f).unwrap_or_default().lines() {
            if let Ok(ev) = serde_json::from_str::<crate::reader::evidence::Evidence>(line) {
                source_of.insert(ev.id.clone(), f.display().to_string());
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
        e.fields.insert("source", source_of.get(&ev.id).cloned().unwrap_or_default());
        if let Some(s) = &ev.slug { e.fields.insert("slug", s.clone()); }
        if let Some(s) = &ev.session { e.fields.insert("session", s.clone()); }
        if let Some(p) = &ev.project { e.fields.insert("project", p.clone()); }
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
        e.fields.insert("source", home.join(".claude/atone/events.jsonl").display().to_string());
        if !s("slug").is_empty() { e.fields.insert("slug", s("slug")); }
        if !s("session_id").is_empty() { e.fields.insert("session", s("session_id")); }
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

/// Proposal id to the weekly finding it was filed from.
fn filed_from() -> BTreeMap<String, String> {
    crate::reader::all_runs()
        .iter()
        .flat_map(|r| r.landed.iter())
        .filter(|l| l.outcome.contains("filed"))
        .filter_map(|l| l.detail.clone().map(|p| (p, l.cluster.clone())))
        .collect()
}

fn proposals(home: &Path, filed: &BTreeMap<String, String>, out: &mut Vec<Entry>) {
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
        if let Some(c) = filed.get(&s("id")) {
            e.fields.insert("finding", c.clone());
            e.related.push(c.clone());
        }
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
        e.related.extend(source.strip_prefix("dream-wake:").map(String::from));
        out.push(e);
    }
}

/// Every scheduled run of an i-dream job, and every full daemon cycle: the
/// "Cycles and runs" list.
fn runs(home: &Path, jobs: &[crate::status::JobStatus], out: &mut Vec<Entry>) {
    let current: BTreeMap<&str, &crate::status::JobStatus> = jobs.iter().map(|j| (j.name.as_str(), j)).collect();
    let logs = home.join(".claude/logs/launchd");
    let mut segments: BTreeMap<String, Vec<(DateTime<Utc>, Vec<String>)>> = BTreeMap::new();
    for r in jsonl(&home.join(".claude/scheduled/history.jsonl")) {
        let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let name = s("name");
        // i-dream's jobs, the current ones and any it has retired.
        if s("ev") != "run" || !(name.contains("i-dream") || name.contains("idream")) {
            continue;
        }
        let job = current.get(name.as_str());
        let log_path = job.map(|j| j.log_path.clone()).unwrap_or_else(|| logs.join(format!("{name}.out.log")).display().to_string());
        let segs = segments.entry(name.clone()).or_insert_with(|| log_segments(Path::new(&log_path)));
        let at = parse_ts(&s("ts"));
        let lines = at.and_then(|t| segs.iter().rev().find(|(start, _)| *start <= t)).map(|(_, l)| l.clone());
        let failed_because = [s("reason"), s("stage")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" at ");
        let mut e = entry(
            "run",
            format!("run:{name}@{}", s("ts")),
            match s("outcome").as_str() {
                "ok" => format!("{name} ran"),
                "failed" => format!("{name} failed"),
                other => format!("{name} ran, outcome not recorded ({})", if other.is_empty() { "none" } else { other }),
            },
            format!(
                "{} · exit {}{}{} · log {}",
                s("outcome"),
                s("exit"),
                if failed_because.is_empty() { String::new() } else { format!(" · {failed_because}") },
                job.map(|j| format!(" · runs {}", j.command)).unwrap_or_else(|| " · a retired job".into()),
                log_path
            ),
        );
        e.at = at;
        e.related = job.map(|j| vec![j.name.clone()]).unwrap_or_default();
        e.fields.insert("job", name.clone());
        e.fields.insert("outcome", s("outcome"));
        e.fields.insert("exit", s("exit"));
        if !failed_because.is_empty() {
            e.fields.insert("reason", failed_because);
        }
        if let Some(l) = lines {
            e.fields.insert("log", l.join("\n"));
        }
        out.push(e);
    }
    // The journal records each consolidation pass; a pass that read nothing
    // is not journaled, so this is fewer than the daemon's cycle count.
    for c in jsonl(&home.join(".claude/subconscious/dreams/journal.jsonl")) {
        let n = |k: &str| c.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        let Some(id) = c.get("cycle_id").or_else(|| c.get("id")).and_then(|v| v.as_str()) else { continue };
        let produced = n("patterns_extracted") + n("associations_found") + n("insights_promoted") > 0;
        let mut e = entry(
            "cycle",
            id,
            if produced { "consolidation, produced something" } else { "consolidation, produced nothing" },
            format!(
                "{} sessions read · {} patterns · {} associations · {} insights · {} tokens",
                n("sessions_analyzed"),
                n("patterns_extracted"),
                n("associations_found"),
                n("insights_promoted"),
                n("tokens_used")
            ),
        );
        e.at = c.get("timestamp").and_then(|v| v.as_str()).and_then(parse_ts);
        e.fields.insert("job", "daemon".into());
        e.fields.insert("outcome", if produced { "produced".into() } else { "nothing".into() });
        e.fields.insert("exit", "".into());
        out.push(e);
    }
}

/// A job's log split into its runs. Each run opens with a
/// `[YYYY-MM-DD HH:MM:SS] running scheduled command` line in local time;
/// only the last 80 lines of a run are kept.
fn log_segments(path: &Path) -> Vec<(DateTime<Utc>, Vec<String>)> {
    let mut out: Vec<(DateTime<Utc>, Vec<String>)> = vec![];
    for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
        let start = line
            .strip_prefix('[')
            .and_then(|r| r.split_once("] running scheduled command"))
            .and_then(|(t, _)| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").ok())
            .and_then(|t| t.and_local_timezone(chrono::Local).single())
            .map(|t| t.with_timezone(&Utc));
        match (start, out.last_mut()) {
            (Some(t), _) => out.push((t, vec![line.to_string()])),
            (None, Some((_, lines))) => lines.push(line.to_string()),
            (None, None) => {}
        }
    }
    for (_, lines) in &mut out {
        if lines.len() > 80 {
            lines.drain(..lines.len() - 80);
        }
    }
    out
}

fn jobs(jobs: &[crate::status::JobStatus], out: &mut Vec<Entry>) {
    for j in jobs {
        out.push(entry("job", &j.name, &j.desc, format!("{} · runs {}", j.schedule, j.command)));
    }
}

/// Sessions that i-dream showed something to, from the firing ledger: which
/// items each one was shown, and when.
fn sessions_at(home: &Path, firings: &Firings, transcripts: &BTreeMap<String, String>, out: &mut Vec<Entry>) {
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
        e.fields.insert("shown", items.len().to_string());
        e.fields.insert("cited", firings.session_cited(&sid));
        e.fields.insert("transcript", transcripts.get(&sid).cloned().unwrap_or_else(|| "transcript not found".into()));
        e.related = items.into_iter().collect();
        out.push(e);
    }
}

/// Every slug and session an evidence row names gets an entry, so no link
/// in the index points at nothing. Slugs and sessions already listed are
/// left as they are.
fn fill_from_evidence(firings: &Firings, transcripts: &BTreeMap<String, String>, out: &mut Vec<Entry>) {
    let have: std::collections::HashSet<(&str, String)> = out.iter().map(|e| (e.kind, e.id.clone())).collect();
    // id to (evidence ids, newest time, projects)
    type Seen = BTreeMap<String, (Vec<String>, Option<DateTime<Utc>>, std::collections::BTreeSet<String>)>;
    let (mut slugs, mut sessions): (Seen, Seen) = Default::default();
    for e in out.iter().filter(|e| e.kind == "evidence") {
        let project = e.fields.get("project").cloned().unwrap_or_default();
        for (key, map, kind) in [("slug", &mut slugs, "slug"), ("session", &mut sessions, "session")] {
            let Some(v) = e.fields.get(key) else { continue };
            if have.contains(&(kind, v.clone())) {
                continue;
            }
            let s = map.entry(v.clone()).or_default();
            s.0.push(e.id.clone());
            s.1 = s.1.max(e.at);
            if !project.is_empty() { s.2.insert(project.clone()); }
        }
    }
    for (slug, (ev, at, _)) in slugs {
        let mut e = entry("slug", &slug, &slug, format!("named by {} evidence row(s); recorded fewer than twice, so it has no weekly trend", ev.len()));
        e.at = at;
        e.fields.insert("total", ev.len().to_string());
        e.related = ev;
        out.push(e);
    }
    for (sid, (ev, at, projects)) in sessions {
        let mut e = entry(
            "session",
            &sid,
            format!("session {}", sid.chars().take(8).collect::<String>()),
            format!("named by {} evidence row(s){}; i-dream showed it nothing live", ev.len(),
                if projects.is_empty() { String::new() } else { format!(" in {}", projects.into_iter().collect::<Vec<_>>().join(", ")) }),
        );
        e.at = at;
        e.fields.insert("shown", "0".into());
        e.fields.insert("cited", firings.session_cited(&sid));
        e.fields.insert("transcript", transcripts.get(&sid).cloned().unwrap_or_else(|| "transcript not found".into()));
        e.related = ev;
        out.push(e);
    }
}

/// "2 down, 1 up", or "none".
fn tally(up: usize, down: usize) -> String {
    match (up, down) {
        (0, 0) => "none".into(),
        (u, 0) => format!("{u} up"),
        (0, d) => format!("{d} down"),
        (u, d) => format!("{d} down, {u} up"),
    }
}

/// What each session said back and what each intention drew: corrections
/// and praise from every prompt's signal, and the votes those cast on the
/// intentions the session had been shown. A session that corrected or voted
/// but was never shown anything live still gets an entry.
fn add_feedback(home: &Path, firings: &Firings, transcripts: &BTreeMap<String, String>, out: &mut Vec<Entry>) {
    let sub = home.join(".claude/subconscious");
    // session -> (corrections, praise)
    let mut said: BTreeMap<String, (usize, usize, Option<DateTime<Utc>>)> = BTreeMap::new();
    for r in jsonl(&sub.join("logs/signals.jsonl")) {
        let ev = r.get("event").unwrap_or(&r);
        let Some(sid) = ev.get("session_id").and_then(|v| v.as_str()) else { continue };
        let corr = ev.get("correction").and_then(|v| v.as_bool()) == Some(true);
        let praise = ev.get("positive").and_then(|v| v.as_bool()) == Some(true);
        if !corr && !praise {
            continue;
        }
        let at = r.get("received_at").and_then(|v| v.as_str()).and_then(parse_ts);
        let e = said.entry(sid.to_string()).or_default();
        if corr { e.0 += 1 }
        if praise { e.1 += 1 }
        e.2 = e.2.max(at);
    }
    // votes by session and by intention: (up, down)
    let (mut by_sid, mut by_intention): (BTreeMap<String, (usize, usize)>, BTreeMap<String, (usize, usize)>) = Default::default();
    for v in jsonl(&sub.join("dreams/insight-feedback.jsonl")) {
        let up = v.get("rating").and_then(|x| x.as_str()) == Some("up");
        for (key, map) in [("sid", &mut by_sid), ("intention_id", &mut by_intention)] {
            if let Some(k) = v.get(key).and_then(|x| x.as_str()) {
                let e = map.entry(k.to_string()).or_default();
                if up { e.0 += 1 } else { e.1 += 1 }
            }
        }
    }
    // intention -> sessions it was shown to
    let mut shown_to: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for f in jsonl(&sub.join("intentions/fired.jsonl")) {
        if let (Some(i), Some(sid)) = (f.get("intention_id").and_then(|v| v.as_str()), f.get("session_id").and_then(|v| v.as_str())) {
            shown_to.entry(i.to_string()).or_default().insert(sid.to_string());
        }
    }
    let have: std::collections::HashSet<String> = out.iter().filter(|e| e.kind == "session").map(|e| e.id.clone()).collect();
    for sid in said.keys().chain(by_sid.keys()) {
        if have.contains(sid) || out.iter().any(|e| e.kind == "session" && &e.id == sid) {
            continue;
        }
        let mut e = entry(
            "session",
            sid,
            format!("session {}", sid.chars().take(8).collect::<String>()),
            "i-dream showed it nothing live; it is listed for what it said back",
        );
        e.at = said.get(sid).and_then(|s| s.2);
        e.fields.insert("shown", "0".into());
        e.fields.insert("cited", firings.session_cited(sid));
        e.fields.insert("transcript", transcripts.get(sid).cloned().unwrap_or_else(|| "transcript not found".into()));
        out.push(e);
    }
    for e in out.iter_mut() {
        match e.kind {
            "session" => {
                let (c, p, _) = said.get(&e.id).cloned().unwrap_or_default();
                e.fields.insert("corrections", if c + p == 0 { "none".into() } else { format!("{c} corrected, {p} praised") });
                let (u, d) = by_sid.get(&e.id).cloned().unwrap_or_default();
                e.fields.insert("votes", tally(u, d));
            }
            "intention" => {
                let (u, d) = by_intention.get(&e.id).cloned().unwrap_or_default();
                e.fields.insert("votes", tally(u, d));
                let seen = shown_to.get(&e.id);
                e.fields.insert("shown to", format!("{} session(s)", seen.map_or(0, |s| s.len())));
                e.related.extend(seen.into_iter().flatten().cloned());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_and_intentions_carry_what_was_said_back() {
        let home = tempfile::tempdir().unwrap();
        let sub = home.path().join(".claude/subconscious");
        std::fs::create_dir_all(sub.join("logs")).unwrap();
        std::fs::create_dir_all(sub.join("dreams")).unwrap();
        std::fs::create_dir_all(sub.join("intentions")).unwrap();
        std::fs::write(sub.join("logs/signals.jsonl"),
            "{\"received_at\":\"2026-10-26T10:00:00Z\",\"event\":{\"session_id\":\"s1\",\"correction\":true,\"positive\":false}}\n\
             {\"event\":{\"session_id\":\"s1\",\"correction\":false,\"positive\":true}}\n\
             {\"event\":{\"session_id\":\"s1\",\"correction\":false,\"positive\":false}}\n").unwrap();
        std::fs::write(sub.join("dreams/insight-feedback.jsonl"),
            "{\"sid\":\"s1\",\"intention_id\":\"i1\",\"rating\":\"down\"}\n{\"sid\":\"s2\",\"intention_id\":\"i1\",\"rating\":\"up\"}\n").unwrap();
        std::fs::write(sub.join("intentions/fired.jsonl"),
            "{\"intention_id\":\"i1\",\"session_id\":\"s1\"}\n{\"intention_id\":\"i1\",\"session_id\":\"s1\"}\n").unwrap();
        let mut out = vec![entry("intention", "i1", "t", "d")];
        add_feedback(home.path(), &Firings::default(), &BTreeMap::new(), &mut out);
        let s1 = out.iter().find(|e| e.id == "s1").unwrap();
        assert_eq!(s1.fields["corrections"], "1 corrected, 1 praised");
        assert_eq!(s1.fields["votes"], "1 down");
        let s2 = out.iter().find(|e| e.id == "s2").unwrap();
        assert_eq!((s2.fields["corrections"].as_str(), s2.fields["votes"].as_str()), ("none", "1 up"));
        let i1 = out.iter().find(|e| e.id == "i1").unwrap();
        assert_eq!(i1.fields["votes"], "1 down, 1 up");
        assert_eq!(i1.fields["shown to"], "1 session(s)");
    }

    #[test]
    fn a_job_log_splits_into_one_segment_per_run() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("job.out.log");
        std::fs::write(
            &log,
            "stray line before any run\n[2026-10-07 04:30:05] running scheduled command (daily)\nfirst a\nfirst b\n\
             [2026-10-08 04:30:05] running scheduled command (daily)\nsecond a\n",
        )
        .unwrap();
        let segs = log_segments(&log);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].1, vec!["[2026-10-07 04:30:05] running scheduled command (daily)", "first a", "first b"]);
        assert!(segs[0].0 < segs[1].0);
        assert_eq!(segs[1].1.len(), 2);
        assert!(log_segments(&dir.path().join("missing.log")).is_empty());
    }

    #[test]
    fn every_slug_and_session_evidence_names_gets_an_entry() {
        let mut ev = entry("evidence", "atone:x", "t", "d");
        ev.fields.insert("slug", "rare-slug".into());
        ev.fields.insert("session", "s-1".into());
        let mut known = entry("slug", "known", "known", "d");
        known.fields.insert("total", "9".into());
        let mut ev2 = entry("evidence", "atone:y", "t", "d");
        ev2.fields.insert("slug", "known".into());
        let mut out = vec![ev, known, ev2];
        let mut f = Firings::default();
        f.by_session.insert("s-1".into(), (1, 3));
        fill_from_evidence(&f, &BTreeMap::new(), &mut out);
        let slugs: Vec<&str> = out.iter().filter(|e| e.kind == "slug").map(|e| e.id.as_str()).collect();
        // The listed slug is not duplicated; the unlisted one is added.
        assert_eq!(slugs, vec!["known", "rare-slug"]);
        let s = out.iter().find(|e| e.kind == "session").unwrap();
        assert_eq!(s.fields["cited"], "cited 1 of 4 items shown");
        assert_eq!(s.fields["transcript"], "transcript not found");
        assert_eq!(Firings::default().session_cited("s-2"), "not checked yet");
    }

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
        sessions_at(h, &f, &BTreeMap::new(), &mut out);
        proposals(h, &BTreeMap::new(), &mut out);
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
