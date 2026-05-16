//! `i-dream domain <subcommand>` — first user-visible surface of the
//! dream-domain plugin system (docs/14-dreaming-plugins.md). Stage 1
//! ships only `list`; further subcommands (info, enable, install, run,
//! …) land with Stage 2+.

use crate::cli::DomainAction;
use crate::config::Config;
use crate::idream_runtime::IDreamRuntime;
use crate::modules::registry::DomainRegistry;
use crate::store::Store;
use anyhow::{Result, bail};
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
        DomainAction::Enable { name } => set_enabled(&name, true, config),
        DomainAction::Disable { name } => set_enabled(&name, false, config),
    }
}

fn set_enabled(name: &str, enabled: bool, config: &Config) -> Result<()> {
    // Reject the action if `name` doesn't refer to a registered external
    // domain. Native modules have their own enable surface; refusing here
    // avoids silently writing meaningless state.
    let store = Store::new(config.data_dir())?;
    let registry = DomainRegistry::boot(config, &store);
    let Some(domain) = registry.get(name) else {
        // Could be an external that was previously disabled — check both
        // the runtime state and the manifest discovery dirs before bailing.
        let rt = IDreamRuntime::load();
        if !rt.enabled.contains_key(name) {
            bail!("Unknown domain '{name}'. Use `i-dream domain list` to see registered domains.");
        }
        let mut rt = rt;
        rt.set_enabled(name, enabled);
        rt.save()?;
        let verb = if enabled { "enabled" } else { "disabled" };
        println!(
            "Domain '{name}' {verb} (was previously disabled — manifest re-discovered on next boot)."
        );
        return Ok(());
    };
    if domain.manifest().consolidation.kind == "native" {
        bail!(
            "Native modules ('{name}') have enable wired to config.modules.<name>.enabled; \
             use that instead. Only external domains support enable/disable via runtime state."
        );
    }
    let mut rt = IDreamRuntime::load();
    rt.set_enabled(name, enabled);
    rt.save()?;
    let verb = if enabled { "enabled" } else { "disabled" };
    println!("Domain '{name}' {verb}.");
    Ok(())
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
            "{:<18} {:<10} {:<10} DESCRIPTION",
            "NAME", "KIND", "CADENCE"
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
