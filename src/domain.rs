//! `i-dream domain <subcommand>` — first user-visible surface of the
//! dream-domain plugin system (docs/14-dreaming-plugins.md). Stage 1
//! ships only `list`; further subcommands (info, enable, install, run,
//! …) land with Stage 2+.

use crate::cli::DomainAction;
use crate::config::Config;
use crate::modules::registry::DomainRegistry;
use crate::store::Store;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
struct DomainListEntry {
    name: String,
    kind: &'static str,
    description: String,
    /// Cadence sourced from the manifest's `[consolidation].cadence` field.
    /// Native modules carry the synthetic `"manifest"` placeholder until
    /// per-domain runtime overrides land (Stage 2 of docs/16).
    cadence: String,
}

pub fn handle(action: DomainAction, config: &Config) -> Result<()> {
    match action {
        DomainAction::List { json } => list(config, json),
    }
}

fn list(config: &Config, as_json: bool) -> Result<()> {
    let store = Store::new(config.data_dir())?;
    let registry = DomainRegistry::boot(config, &store);

    let entries: Vec<DomainListEntry> = registry
        .iter()
        .map(|d| {
            let m = d.manifest();
            DomainListEntry {
                name: d.name().to_string(),
                kind: if m.consolidation.kind == "native" {
                    "native"
                } else {
                    "external"
                },
                description: m.domain.description.clone(),
                cadence: m.consolidation.cadence.clone(),
            }
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string(&entries)?);
    } else {
        if entries.is_empty() {
            println!("(no domains registered)");
            return Ok(());
        }
        println!(
            "{:<18} {:<10} {:<10} {}",
            "NAME", "KIND", "CADENCE", "DESCRIPTION"
        );
        for e in &entries {
            println!(
                "{:<18} {:<10} {:<10} {}",
                e.name, e.kind, e.cadence, e.description
            );
        }
    }
    Ok(())
}
