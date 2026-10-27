//! Naming what the join found, with one bounded model call.
//!
//! The model sees each forwarded cluster with its evidence excerpts and says
//! what kind of thing it is (a repeat, a structural need, a drift, or noise),
//! gives it a title a person would recognise, and may draft a proposal. Its
//! output is checked against the batch: an evidence id the cluster does not
//! own is dropped, and an item left with none is rejected (docs/32 §4).

use super::evidence::Evidence;
use super::join::Cluster;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// What one cluster was named as.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Named {
    pub cluster: String,
    /// Other clusters this problem absorbs; their evidence is citable too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged: Vec<String>,
    /// repeat, structural-need, drift or noise.
    pub kind: String,
    /// The problem as the owner would say it happened to them.
    pub title: String,
    /// The machine-facing reason, kept for the backlog proposal body.
    #[serde(default, deserialize_with = "null_default")]
    pub why: String,
    #[serde(default, deserialize_with = "null_default")]
    pub evidence_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposal>,
    /// Dated instances. The model writes `what`; the page takes the date and
    /// project from the evidence row, never from the model.
    #[serde(default, deserialize_with = "null_default", skip_serializing_if = "Vec::is_empty")]
    pub happened: Vec<Instance>,
    /// Why it keeps happening, in owner voice, no hook ids or script names.
    #[serde(default, deserialize_with = "null_default")]
    pub why_repeats: String,
    /// The proposed fix in plain words, no paths.
    #[serde(default, deserialize_with = "null_default")]
    pub fix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<Guard>,
}

/// One time the problem happened, tied to the evidence row that records it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Instance {
    pub evidence_id: String,
    pub what: String,
}

/// What tries to stop the problem today, and whether it works.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Guard {
    /// A hook id from the 14-day action list, or None for "nothing".
    #[serde(default)]
    pub hook_id: Option<String>,
    /// The check named in owner words: "A check on done and works claims".
    #[serde(default, deserialize_with = "null_default")]
    pub plain: String,
    /// yes, partly or no.
    #[serde(default, deserialize_with = "null_default")]
    pub verdict: String,
    /// What it misses, one clause, no numbers (the page adds the counts).
    #[serde(default, deserialize_with = "null_default")]
    pub gap: String,
}

/// The model writes `null` for a field that has nothing to say (a noise item
/// has no fix), and that means the same as leaving the field out.
pub(crate) fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    /// A file path, hook id, rule name or mute file the change applies to.
    pub target: String,
    pub change: String,
}

/// The model's whole answer before validation.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawNaming {
    #[serde(default)]
    pub items: Vec<Named>,
    /// Two or three sentences for the top of the page: the habits behind the
    /// problems and the state of the checks meant to stop them.
    #[serde(default)]
    pub summary: String,
    /// The weekly run may amend its own process (ruling D7).
    #[serde(default)]
    pub process_note: Option<String>,
    /// Items that were not the documented shape, each with the reason, so
    /// one malformed item costs that item and not the week's run.
    #[serde(skip)]
    pub unparsed: Vec<String>,
}

/// The answer's envelope, with each item kept raw so items parse one by one.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default, deserialize_with = "null_default")]
    items: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "null_default")]
    summary: String,
    #[serde(default)]
    process_note: Option<String>,
}

/// Read the model's answer: a ```json block or bare JSON, either the
/// documented object or a bare list of items. Fails only when the answer as
/// a whole is unreadable, with serde's reason; a bad item is kept in
/// `unparsed` instead.
pub fn parse_answer(content: &str) -> Result<RawNaming, String> {
    let json = crate::modules::parse_json_codeblock(content).unwrap_or_else(|| content.trim().to_string());
    let env = match serde_json::from_str::<Envelope>(&json) {
        Ok(env) => env,
        Err(obj_err) => match serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            Ok(items) => Envelope { items, summary: String::new(), process_note: None },
            Err(_) => return Err(obj_err.to_string()),
        },
    };
    let mut raw = RawNaming { summary: env.summary, process_note: env.process_note, ..Default::default() };
    for (i, v) in env.items.into_iter().enumerate() {
        let label = v.get("cluster").and_then(|c| c.as_str()).map(str::to_string).unwrap_or_else(|| format!("item {i}"));
        match serde_json::from_value::<Named>(v) {
            Ok(item) => raw.items.push(item),
            Err(e) => raw.unparsed.push(format!("{label}: not the documented shape ({e})")),
        }
    }
    Ok(raw)
}

/// What validation kept and why it dropped the rest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Validated {
    pub items: Vec<Named>,
    pub rejected: Vec<String>,
    pub dropped_ids: usize,
    /// The model's lead paragraph for the page.
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_note: Option<String>,
}

const KINDS: [&str; 4] = ["repeat", "structural-need", "drift", "noise"];

/// The instructions the weekly run reads, written on first use to
/// `reader/PROCESS.md` so later runs (and the owner) can amend them.
pub const DEFAULT_PROCESS: &str = r#"# How the weekly reader names what it found

You are reading clusters a deterministic join found across the owner's local signals:
recorded mistakes (atone), affirmed good behaviour (affirm), session pins, gcc
improvement proposals, core-dumps with unfinished items, corrections the owner typed,
hook telemetry and skill use. Each cluster lists its evidence as
`id | domain | provenance | date | project | excerpt`.

For each cluster, decide what it is:
- repeat: the same lesson keeps coming back from independent places.
- structural-need: the evidence says a hook, gate, skill fix, rule reshape or tool
  default is missing. Text reminders have already failed for these; name the
  structural change.
- drift: something is getting worse week over week.
- noise: the join matched on something incidental. Saying so is useful.

The answer's fields and its JSON shape are given after the clusters, in the prompt
itself; follow that schema. The owner-facing fields are written for the person Claude
worked for. `why` is one sentence for the backlog that cites what the evidence shows.
Cite only evidence ids listed under that cluster (or a cluster you merge).
Where a change would help, draft one: `target` is the file, hook id, rule or mute file
it touches, and `change` is one sentence. Do not propose more text injected into
sessions; that lever has been measured and does not move these counts.

If this process itself is missing something you needed, say so in `process_note`;
it is appended to this file and read next week.

## Amendments
"#;

/// The batch the model sees: each cluster with at most `per_cluster` evidence
/// lines, newest first.
pub fn render_batch(clusters: &[Cluster], rows: &[Evidence], per_cluster: usize) -> String {
    let by_id: HashMap<&str, &Evidence> = rows.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut out = String::new();
    for c in clusters {
        out.push_str(&format!(
            "## {} (found by {}, score {:.1})\n{}\n",
            c.id,
            c.kind.label().replace('-', " "),
            c.score,
            c.summary
        ));
        let mut ev: Vec<&Evidence> = c.evidence.iter().filter_map(|id| by_id.get(id.as_str()).copied()).collect();
        ev.sort_by(|a, b| b.ts.cmp(&a.ts));
        for e in ev.iter().take(per_cluster) {
            out.push_str(&format!(
                "- {} | {} | {} | {} | {} | {}\n",
                e.id,
                e.domain,
                e.provenance,
                e.ts.format("%Y-%m-%d"),
                e.project.as_deref().unwrap_or("-"),
                e.text
            ));
        }
        if c.evidence.len() > per_cluster {
            out.push_str(&format!("- (+{} more)\n", c.evidence.len() - per_cluster));
        }
        out.push('\n');
    }
    out
}

/// Keep what the batch can vouch for. Unknown clusters, unknown kinds, empty
/// titles and items whose every evidence id is foreign are rejected; foreign
/// ids inside an otherwise good item are dropped and counted.
///
/// A cluster belongs to one problem only, whether that problem names it or
/// merges it. Named clusters are collected in a first pass so a merge of a
/// cluster that a later item names is caught as a double claim.
pub fn validate(raw: RawNaming, clusters: &[Cluster]) -> Validated {
    let owned: HashMap<&str, HashSet<&str>> = clusters
        .iter()
        .map(|c| (c.id.as_str(), c.evidence.iter().map(String::as_str).collect()))
        .collect();
    let mut v = Validated {
        process_note: raw.process_note.filter(|n| !n.trim().is_empty()),
        summary: raw.summary.trim().to_string(),
        rejected: raw.unparsed,
        ..Default::default()
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut named: Vec<Named> = vec![];
    for item in raw.items {
        if !owned.contains_key(item.cluster.as_str()) {
            v.rejected.push(format!("{}: not a cluster in this batch", item.cluster));
            continue;
        }
        if !KINDS.contains(&item.kind.as_str()) {
            v.rejected.push(format!("{}: unknown kind '{}'", item.cluster, item.kind));
            continue;
        }
        if item.title.trim().is_empty() {
            v.rejected.push(format!("{}: no title", item.cluster));
            continue;
        }
        if !seen.insert(item.cluster.clone()) {
            v.rejected.push(format!("{}: named twice", item.cluster));
            continue;
        }
        named.push(item);
    }
    'items: for mut item in named {
        // Claims land only when every merge is good, so a rejected item
        // leaves its other merges free for a later item.
        let mut claims: HashSet<String> = HashSet::new();
        for m in &item.merged {
            if !owned.contains_key(m.as_str()) {
                v.rejected.push(format!("{}: merges unknown {m}", item.cluster));
                continue 'items;
            }
            if seen.contains(m) || !claims.insert(m.clone()) {
                v.rejected.push(format!("{}: merges {m}, already claimed", item.cluster));
                continue 'items;
            }
        }
        let ids: HashSet<&str> = std::iter::once(item.cluster.as_str())
            .chain(item.merged.iter().map(String::as_str))
            .filter_map(|c| owned.get(c))
            .flat_map(|s| s.iter().copied())
            .collect();
        let before = item.evidence_ids.len();
        item.evidence_ids.retain(|id| ids.contains(id.as_str()));
        v.dropped_ids += before - item.evidence_ids.len();
        if item.evidence_ids.is_empty() {
            v.rejected.push(format!("{}: cites no evidence the cluster owns", item.cluster));
            continue;
        }
        let before = item.happened.len();
        item.happened
            .retain(|h| ids.contains(h.evidence_id.as_str()) && !h.what.trim().is_empty());
        v.dropped_ids += before - item.happened.len();
        if let Some(g) = &item.guard
            && !["yes", "partly", "no"].contains(&g.verdict.as_str())
        {
            item.guard = None;
        }
        // The owner-facing fields are what the page shows; filing does not need them.
        if item.kind != "noise" && (item.happened.is_empty() || item.fix.trim().is_empty()) {
            v.rejected.push(format!("{}: no dated instance or no plain fix", item.cluster));
            continue;
        }
        if let Some(p) = &item.proposal
            && (p.target.trim().is_empty() || p.change.trim().is_empty())
        {
            item.proposal = None;
        }
        seen.extend(claims);
        v.items.push(item);
    }
    v
}

/// The script names a hook command runs, by basename: "~/x/guard.sh --y" → "guard.sh".
pub fn hook_scripts(settings: &serde_json::Value) -> HashSet<String> {
    fn walk(v: &serde_json::Value, out: &mut HashSet<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, x) in m {
                    if k == "command"
                        && let Some(cmd) = x.as_str()
                    {
                        out.extend(script_names(cmd));
                    } else {
                        walk(x, out);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = HashSet::new();
    if let Some(h) = settings.get("hooks") {
        walk(h, &mut out);
    }
    out
}

fn script_names(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || "\"'`(),;".contains(c))
        .filter(|t| t.ends_with(".sh") || t.ends_with(".py"))
        .filter_map(|t| t.rsplit('/').next().map(str::to_string))
        .collect()
}

/// Drop proposals whose premise a lookup disproves, and say why in `rejected`.
/// The finding itself stays; only the wrong fix goes.
pub fn ground(v: &mut Validated, registered: &HashSet<String>, exists: impl Fn(&str) -> bool) {
    for item in &mut v.items {
        let Some(p) = &item.proposal else { continue };
        let change = p.change.to_lowercase();
        let wires = ["register", "wire", "add "].iter().any(|w| change.contains(w));
        if wires
            && let Some(s) = script_names(&p.change).into_iter().find(|s| registered.contains(s))
        {
            v.rejected.push(format!("{}: proposal dropped, {s} is already registered in settings.json", item.cluster));
            item.proposal = None;
            continue;
        }
        let target = p.target.trim();
        let single_path = !target.contains(' ') && (target.starts_with('/') || target.starts_with('~'));
        let creates = ["create", "add ", "new "].iter().any(|w| change.contains(w));
        if single_path && !creates && !exists(target) {
            v.rejected.push(format!("{}: proposal dropped, target {target} does not exist", item.cluster));
            item.proposal = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::join::ClusterKind;
    use chrono::Utc;

    fn cluster(id: &str, ev: &[&str]) -> Cluster {
        Cluster {
            id: id.into(),
            kind: ClusterKind::Repeat,
            key: "k".into(),
            summary: "s".into(),
            evidence: ev.iter().map(|s| s.to_string()).collect(),
            domains: vec![],
            provenances: vec![],
            first_ts: Utc::now(),
            last_ts: Utc::now(),
            score: 1.0,
            weekly: vec![],
        }
    }

    /// A well-formed item: its first cited id is also its one dated instance.
    fn item(cluster: &str, kind: &str, ids: &[&str]) -> Named {
        Named {
            cluster: cluster.into(),
            kind: kind.into(),
            title: "t".into(),
            why: "w".into(),
            evidence_ids: ids.iter().map(|s| s.to_string()).collect(),
            happened: ids
                .first()
                .map(|id| Instance { evidence_id: id.to_string(), what: "it happened".into() })
                .into_iter()
                .collect(),
            fix: "a plain fix".into(),
            ..Default::default()
        }
    }

    /// The 2026w44 answer wrote null for a noise item's empty fields, and the
    /// whole week failed on it. Null reads as empty, and an item that is
    /// still the wrong shape costs only itself.
    #[test]
    fn null_fields_and_one_bad_item_do_not_sink_the_answer() {
        let answer = r#"```json
{"summary": "s", "items": [
  {"cluster": "c-1", "kind": "noise", "title": "t", "happened": [], "why_repeats": null,
   "fix": null, "guard": {"hook_id": null, "plain": null, "verdict": "no", "gap": null},
   "why": "w", "evidence_ids": ["atone:a"], "proposal": null},
  {"cluster": "c-2", "kind": "repeat", "title": 7, "evidence_ids": []}
], "process_note": null}
```"#;
        let raw = parse_answer(answer).unwrap();
        assert_eq!(raw.items.len(), 1);
        assert_eq!(raw.items[0].fix, "");
        assert_eq!(raw.items[0].guard.as_ref().unwrap().plain, "");
        assert_eq!(raw.unparsed.len(), 1);
        assert!(raw.unparsed[0].starts_with("c-2: "), "{:?}", raw.unparsed);

        let cs = vec![cluster("c-1", &["atone:a"]), cluster("c-2", &["atone:b"])];
        let v = validate(raw, &cs);
        assert!(v.rejected.iter().any(|r| r.starts_with("c-2: ")), "{:?}", v.rejected);

        let err = parse_answer("not json at all").unwrap_err();
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn every_kept_evidence_id_belongs_to_its_cluster() {
        let cs = vec![cluster("c-1", &["atone:a", "atone:b"]), cluster("c-2", &["pinned:p"])];
        let raw = RawNaming {
            items: vec![
                item("c-1", "repeat", &["atone:a", "pinned:p", "made:up"]),
                item("c-2", "noise", &["atone:a"]),
                item("c-9", "repeat", &["atone:a"]),
                item("c-1", "bogus", &["atone:a"]),
            ],
            process_note: Some("  ".into()),
            ..Default::default()
        };
        let v = validate(raw, &cs);
        assert_eq!(v.items.len(), 1);
        assert_eq!(v.items[0].evidence_ids, vec!["atone:a".to_string()]);
        assert_eq!(v.dropped_ids, 3, "two foreign ids in c-1, one in c-2");
        assert_eq!(v.rejected.len(), 3);
        assert!(v.process_note.is_none(), "a blank note is no note");
        for it in &v.items {
            let owner = cs.iter().find(|c| c.id == it.cluster).unwrap();
            assert!(it.evidence_ids.iter().all(|id| owner.evidence.contains(id)));
        }
    }

    #[test]
    fn a_half_empty_proposal_is_removed_not_kept() {
        let cs = vec![cluster("c-1", &["atone:a"])];
        let mut it = item("c-1", "structural-need", &["atone:a"]);
        it.proposal = Some(Proposal { target: "".into(), change: "x".into() });
        let v = validate(RawNaming { items: vec![it], ..Default::default() }, &cs);
        assert!(v.items[0].proposal.is_none());
    }

    #[test]
    fn merge_claims_each_cluster_once() {
        let cs = vec![
            cluster("c-1", &["atone:a"]),
            cluster("c-2", &["atone:b"]),
            cluster("c-3", &["atone:c"]),
            cluster("c-4", &["atone:d"]),
        ];
        let merging = |c: &str, ids: &[&str], merged: &[&str]| {
            let mut it = item(c, "repeat", ids);
            it.merged = merged.iter().map(|s| s.to_string()).collect();
            it
        };
        let raw = RawNaming {
            items: vec![
                // Good: c-1 absorbs c-2, so c-2's evidence becomes citable.
                merging("c-1", &["atone:a", "atone:b"], &["c-2"]),
                // Merges a cluster the batch does not have.
                merging("c-3", &["atone:c"], &["c-9"]),
                // Merges c-2 again, already claimed by c-1.
                merging("c-4", &["atone:d"], &["c-2"]),
                // A second naming of c-3 is a double claim too.
                merging("c-3", &["atone:c"], &["c-1"]),
            ],
            ..Default::default()
        };
        let v = validate(raw, &cs);
        assert_eq!(v.items.len(), 1, "rejected: {:?}", v.rejected);
        assert_eq!(v.items[0].evidence_ids, vec!["atone:a".to_string(), "atone:b".to_string()]);
        assert!(v.rejected.iter().any(|r| r.contains("merges unknown c-9")), "{:?}", v.rejected);
        assert!(v.rejected.iter().any(|r| r == "c-4: merges c-2, already claimed"), "{:?}", v.rejected);
        assert!(v.rejected.iter().any(|r| r == "c-3: named twice"), "{:?}", v.rejected);

        // A named cluster cannot be merged by an item that comes before it.
        let raw = RawNaming {
            items: vec![merging("c-1", &["atone:a"], &["c-2"]), item("c-2", "repeat", &["atone:b"])],
            ..Default::default()
        };
        let v = validate(raw, &cs);
        assert_eq!(v.items.len(), 1);
        assert_eq!(v.items[0].cluster, "c-2");
        assert_eq!(v.rejected, vec!["c-1: merges c-2, already claimed".to_string()]);
    }

    #[test]
    fn a_page_item_needs_an_owned_instance_and_a_fix() {
        let cs = vec![cluster("c-1", &["atone:a"]), cluster("c-2", &["atone:b"])];
        let mut foreign = item("c-1", "repeat", &["atone:a"]);
        foreign.happened = vec![Instance { evidence_id: "atone:zz".into(), what: "x".into() }];
        let mut no_fix = item("c-2", "repeat", &["atone:b"]);
        no_fix.fix = " ".into();
        let v = validate(RawNaming { items: vec![foreign, no_fix], ..Default::default() }, &cs);
        assert!(v.items.is_empty());
        assert_eq!(v.rejected.len(), 2);
        assert_eq!(v.dropped_ids, 1, "the foreign instance is counted as dropped");
    }

    #[test]
    fn the_batch_shows_newest_evidence_and_counts_the_rest() {
        let now = Utc::now();
        let rows: Vec<Evidence> = (0..5)
            .map(|i| Evidence {
                id: format!("atone:{i}"),
                domain: "atone".into(),
                ts: now - chrono::Duration::days(i),
                provenance: "human".into(),
                slug: Some("s".into()),
                session: None,
                project: None,
                paths: vec![],
                text: format!("event {i}"),
                fires_week: None,
            })
            .collect();
        let c = cluster("c-1", &["atone:0", "atone:1", "atone:2", "atone:3", "atone:4"]);
        let b = render_batch(&[c], &rows, 2);
        assert!(b.contains("atone:0") && b.contains("atone:1"));
        assert!(!b.contains("atone:4 |"));
        assert!(b.contains("(+3 more)"));
    }

    fn proposed(cluster: &str, target: &str, change: &str) -> Named {
        let mut it = item(cluster, "structural-need", &["atone:a"]);
        it.proposal = Some(Proposal { target: target.into(), change: change.into() });
        it
    }

    #[test]
    fn hook_scripts_reads_every_command_basename() {
        let s: serde_json::Value = serde_json::from_str(
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"~/.claude/scripts/hooks/declared-ready-stop.sh"}]}],
               "PreToolUse":[{"hooks":[{"command":"python3 ~/.claude/g/guard-settings-write.py --strict"}]}]},
               "statusLine":{"command":"~/not-a-hook.sh"}}"#,
        )
        .unwrap();
        let got = hook_scripts(&s);
        assert!(got.contains("declared-ready-stop.sh"));
        assert!(got.contains("guard-settings-write.py"));
        assert!(!got.contains("not-a-hook.sh"), "only the hooks block counts");
    }

    /// The four 2026w41 proposals that asked to register a hook that was
    /// already registered, plus the cases that must survive.
    #[test]
    fn ground_drops_fixes_a_lookup_disproves_and_keeps_the_rest() {
        let registered: HashSet<String> = ["named-next-work-stop.sh", "declared-ready-stop.sh"].iter().map(|s| s.to_string()).collect();
        let mut v = Validated {
            items: vec![
                proposed("c-1", "~/.claude/settings.json Stop hooks",
                    "Register named-next-work-stop.sh as a Stop hook — the script exists but is not wired."),
                proposed("c-2", "~/.claude/settings.json Stop hooks",
                    "Register brand-new-gate.sh as a Stop hook."),
                proposed("c-3", "~/.claude/rules/missing.md", "Tighten the precheck wording."),
                proposed("c-4", "~/.claude/rules/missing.md", "Create a rule for this pattern."),
                proposed("c-5", "skills/gcc-mods/hooks/register.tsx", "Use mode insert, not replace."),
            ],
            ..Default::default()
        };
        ground(&mut v, &registered, |p| !p.contains("missing"));
        let kept: Vec<bool> = v.items.iter().map(|i| i.proposal.is_some()).collect();
        assert_eq!(kept, vec![false, true, false, true, true]);
        assert_eq!(v.items.len(), 5, "findings stay; only the wrong fixes go");
        assert!(v.rejected[0].contains("named-next-work-stop.sh is already registered"));
        assert!(v.rejected[1].contains("does not exist"));
    }

    /// Replays the real 2026w41 run against the owner's real settings.json.
    /// Run with `cargo test -- --ignored replay_w41`; prints what it drops.
    #[test]
    #[ignore]
    fn replay_w41() {
        let home = dirs::home_dir().unwrap();
        let run: crate::reader::WeeklyRun = serde_json::from_str(
            &std::fs::read_to_string(home.join(".claude/i-dream/reader/runs/2026w41/run.json")).unwrap(),
        )
        .unwrap();
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap();
        let mut v = Validated { items: run.items.clone(), ..Default::default() };
        ground(&mut v, &hook_scripts(&settings), |p| {
            crate::config::expand_tilde(std::path::Path::new(p)).exists()
        });
        for r in &v.rejected {
            println!("{r}");
        }
        let kept = v.items.iter().filter(|i| i.proposal.is_some()).count();
        println!("kept {kept} of {}", v.items.len());
    }
}
