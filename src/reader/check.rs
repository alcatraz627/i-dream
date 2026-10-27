//! Holding the reader's account of each check to the check's own source.
//!
//! The naming call says why a problem keeps happening and what the check
//! meant to stop it misses, but it sees only a hook's name and its counts, so
//! it guesses. This second, smaller call shows it the header of every hook it
//! credited and asks it to correct whatever that source contradicts, and to
//! drop any case that is not the same problem as the rest.

use super::name::{Guard, Named, null_default};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How much of a hook the model sees. Hook headers in this account carry
/// their purpose, scope and response there, and the first 80 lines hold it.
const HEADER_LINES: usize = 80;

/// The script a hook's telemetry id belongs to, and its header. A file named
/// for the id wins; otherwise the first script that logs under that id.
pub fn hook_source(dir: &Path, hook_id: &str) -> Option<(PathBuf, String)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !p.to_string_lossy().contains(".test."))
        .collect();
    files.sort();
    let stem = |p: &Path| p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let named = [hook_id.to_string(), format!("guard-{hook_id}"), format!("{hook_id}-stop"), format!("{hook_id}-gate")];
    let logs_as = |body: &str| {
        [format!("--hook {hook_id} "), format!("--hook {hook_id}\n"), format!("--hook \"{hook_id}\""), format!("--hook '{hook_id}'")]
            .iter()
            .any(|m| body.contains(m.as_str()))
    };
    let pick = files
        .iter()
        .find(|p| named.contains(&stem(p)))
        .cloned()
        .or_else(|| files.iter().find(|p| std::fs::read_to_string(p).is_ok_and(|b| logs_as(&b))).cloned())?;
    let body = std::fs::read_to_string(&pick).ok()?;
    let header = body.lines().take(HEADER_LINES).collect::<Vec<_>>().join("\n");
    Some((pick, header))
}

/// The prompt for the check call: the items as named, then each credited
/// hook's source. None when no item credits a hook whose source was found.
pub fn prompt(items: &[Named], hooks_dir: &Path) -> Option<String> {
    let mut seen = HashSet::new();
    let mut sources = String::new();
    for id in items.iter().filter_map(|n| n.guard.as_ref()?.hook_id.as_deref()) {
        if !seen.insert(id.to_string()) {
            continue;
        }
        if let Some((path, header)) = hook_source(hooks_dir, id) {
            sources.push_str(&format!("### {id} ({})\n```\n{header}\n```\n\n", path.display()));
        }
    }
    if sources.is_empty() {
        return None;
    }
    let named = serde_json::to_string_pretty(items).ok()?;
    Some(format!(
        "These problems were named from evidence alone, without reading the checks they credit. \
         Below them is the source header of each credited check.\n\n\
         ## The problems as named\n```json\n{named}\n```\n\n## The credited checks\n{sources}\
         ## What to do\n\
         For each problem:\n\
         - Correct `why_repeats`, `fix` and `guard` wherever the check's source contradicts them: \
         what it looks at, where it runs, whether it blocks or only warns. If the credited check \
         cannot see this problem at all, say that plainly, and aim `fix` at what is actually missing.\n\
         - If a case in `happened` is a different problem from the others, list it in `drop` with \
         one sentence on why.\n\
         - Leave a field out when it is already right.\n\
         Keep the same voice rules as before: owner-facing fields are plain sentences about what \
         Claude did, with no hook ids, script names, slugs or numbers.\n\n\
         Answer with one ```json block: {{\"items\": [{{\"cluster\": \"...\", \"why_repeats\": \"...\", \
         \"fix\": \"...\", \"guard\": {{\"hook_id\": \"...\" or null, \"plain\": \"...\", \"verdict\": \
         \"yes|partly|no\", \"gap\": \"...\"}}, \"drop\": [{{\"evidence_id\": \"...\", \"why\": \"...\"}}]}}]}}"
    ))
}

#[derive(Debug, Default, Deserialize)]
struct Answer {
    #[serde(default, deserialize_with = "null_default")]
    items: Vec<Checked>,
}

#[derive(Debug, Default, Deserialize)]
struct Checked {
    cluster: String,
    #[serde(default, deserialize_with = "null_default")]
    why_repeats: String,
    #[serde(default, deserialize_with = "null_default")]
    fix: String,
    #[serde(default)]
    guard: Option<Guard>,
    #[serde(default, deserialize_with = "null_default")]
    drop: Vec<Dropped>,
}

#[derive(Debug, Default, Deserialize)]
struct Dropped {
    evidence_id: String,
    #[serde(default, deserialize_with = "null_default")]
    why: String,
}

/// Fold the check call's corrections into the items. Returns one line per
/// case it took out, for the run's rejected list. A case is taken out only if
/// the problem keeps at least one other.
pub fn apply(items: &mut [Named], answer: &str) -> Result<Vec<String>, String> {
    let json = crate::modules::parse_json_codeblock(answer).unwrap_or_else(|| answer.trim().to_string());
    let parsed: Answer = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let mut notes = vec![];
    for c in parsed.items {
        let Some(item) = items.iter_mut().find(|n| n.cluster == c.cluster) else { continue };
        if !c.why_repeats.trim().is_empty() {
            item.why_repeats = c.why_repeats;
        }
        if !c.fix.trim().is_empty() {
            item.fix = c.fix;
        }
        if let Some(g) = c.guard {
            item.guard = Some(g);
        }
        for d in c.drop {
            if item.happened.len() <= 1 {
                break;
            }
            let before = item.happened.len();
            item.happened.retain(|h| h.evidence_id != d.evidence_id);
            if item.happened.len() < before {
                item.evidence_ids.retain(|e| e != &d.evidence_id);
                notes.push(format!("{}: took out {} as a different problem ({})", item.cluster, d.evidence_id, d.why.trim()));
            }
        }
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::name::Instance;

    fn hooks_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("doc-voice-gate.py"), "\"\"\"Checks docs under docs/ only.\"\"\"\n").unwrap();
        std::fs::write(d.path().join("doc-voice-gate.test.sh"), "test\n").unwrap();
        std::fs::write(
            d.path().join("guard-git-push.sh"),
            "# Holds pushes to main.\nbash warn-log.sh --hook push-gate --action block\n",
        )
        .unwrap();
        d
    }

    fn item(cluster: &str, hook: &str, cases: &[&str]) -> Named {
        Named {
            cluster: cluster.into(),
            kind: "repeat".into(),
            title: "t".into(),
            evidence_ids: cases.iter().map(|s| s.to_string()).collect(),
            happened: cases.iter().map(|c| Instance { evidence_id: c.to_string(), what: "w".into() }).collect(),
            why_repeats: "guessed".into(),
            fix: "guessed fix".into(),
            guard: Some(Guard { hook_id: Some(hook.into()), plain: "p".into(), verdict: "partly".into(), gap: "g".into() }),
            ..Default::default()
        }
    }

    #[test]
    fn finds_a_hook_by_its_name_or_by_what_it_logs_as() {
        let d = hooks_dir();
        let (p, h) = hook_source(d.path(), "doc-voice-gate").unwrap();
        assert!(p.ends_with("doc-voice-gate.py") && h.contains("docs/ only"));
        let (p, _) = hook_source(d.path(), "push-gate").unwrap();
        assert!(p.ends_with("guard-git-push.sh"), "{}", p.display());
        assert!(hook_source(d.path(), "no-such-hook").is_none());
    }

    #[test]
    fn the_prompt_carries_each_credited_source_once() {
        let d = hooks_dir();
        let items = vec![item("c-1", "doc-voice-gate", &["a:1"]), item("c-2", "doc-voice-gate", &["a:2"])];
        let p = prompt(&items, d.path()).unwrap();
        assert_eq!(p.matches("### doc-voice-gate").count(), 1);
        assert!(p.contains("docs/ only"));
        assert!(prompt(&[item("c-3", "no-such-hook", &["a:3"])], d.path()).is_none());
    }

    #[test]
    fn corrections_land_and_a_lone_case_is_never_taken_out() {
        let mut items = vec![item("c-1", "doc-voice-gate", &["a:1", "a:2"]), item("c-2", "push-gate", &["a:3"])];
        let answer = r#"```json
{"items": [
  {"cluster": "c-1", "why_repeats": "The check never sees PR bodies.", "fix": null,
   "drop": [{"evidence_id": "a:2", "why": "a directness problem"}]},
  {"cluster": "c-2", "guard": {"hook_id": null, "plain": "Nothing", "verdict": "no", "gap": null},
   "drop": [{"evidence_id": "a:3", "why": "only case"}]}
]}
```"#;
        let notes = apply(&mut items, answer).unwrap();
        assert_eq!(items[0].why_repeats, "The check never sees PR bodies.");
        assert_eq!(items[0].fix, "guessed fix", "a null fix leaves the old one");
        assert_eq!(items[0].evidence_ids, vec!["a:1".to_string()]);
        assert_eq!(items[1].guard.as_ref().unwrap().hook_id, None);
        assert_eq!(items[1].happened.len(), 1, "the only case stays");
        assert_eq!(notes.len(), 1);
        assert!(apply(&mut items, "nope").is_err());
    }
}
