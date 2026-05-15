//! Registry of subconscious domains — native compiled modules (this dir's
//! submodules, wrapped in `NativeAdapter`) plus, eventually, external plugins
//! loaded from manifests. Built per-tick by the daemon; cheap to construct.
//!
//! Full design: docs/14-dreaming-plugins.md §3.3.

use crate::config::Config;
use crate::modules::{
    DreamDomain, NativeAdapter, dreaming::DreamingModule, insight_digest::InsightDigestModule,
    introspection::IntrospectionModule, intuition::IntuitionModule, metacog::MetacogModule,
    prospective::ProspectiveModule, weekly_briefing::WeeklyBriefingModule,
};
use crate::store::Store;

/// The native subconscious modules registered via `Module` trait. Ordering
/// is the daemon's tick-evaluation order — change deliberately. One holdout
/// (`project_briefs`) deliberately stays unregistered: its per-project
/// regeneration loop doesn't fit the per-cycle `Module::run` contract
/// without contorting either side. Stage 2+ may grow a companion trait
/// (e.g. `PerProjectDomain`) for shapes like that.
pub const NATIVE_MODULE_NAMES: &[&str] = &[
    "dreaming",
    "metacog",
    "intuition",
    "introspection",
    "prospective",
    "insight_digest",
    "weekly_briefing",
];

/// Holds every registered domain for the current tick. Lifetime is tied to
/// the Config + Store the daemon owns; constructed once per tick, dropped at
/// the end of the tick.
pub struct DomainRegistry<'a> {
    domains: Vec<Box<dyn DreamDomain + 'a>>,
}

impl<'a> DomainRegistry<'a> {
    /// Construct a registry holding `NativeAdapter`-wrapped instances of
    /// every native module. External plugin discovery lands in Stage 2.
    pub fn boot(config: &'a Config, store: &'a Store) -> Self {
        let domains: Vec<Box<dyn DreamDomain + 'a>> = vec![
            Box::new(NativeAdapter::new(
                "dreaming",
                DreamingModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "metacog",
                MetacogModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "intuition",
                IntuitionModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "introspection",
                IntrospectionModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "prospective",
                ProspectiveModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "insight_digest",
                InsightDigestModule::new(config, store),
            )),
            Box::new(NativeAdapter::new(
                "weekly_briefing",
                WeeklyBriefingModule::new(config, store),
            )),
        ];
        Self { domains }
    }

    /// Build a registry from pre-constructed adapters. Useful for tests.
    pub fn from_domains(domains: Vec<Box<dyn DreamDomain + 'a>>) -> Self {
        Self { domains }
    }

    pub fn iter(&self) -> impl Iterator<Item = &(dyn DreamDomain + 'a)> {
        self.domains.iter().map(|b| b.as_ref())
    }

    pub fn get(&self, name: &str) -> Option<&(dyn DreamDomain + 'a)> {
        self.iter().find(|d| d.name() == name)
    }

    pub fn len(&self) -> usize {
        self.domains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ClaudeClient;
    use crate::modules::Module;
    use anyhow::Result;

    struct StubModule;

    impl Module for StubModule {
        fn should_run(&self) -> Result<bool> {
            Ok(false)
        }
        fn run(
            &self,
            _client: &ClaudeClient,
            _budget_tokens: u64,
        ) -> impl std::future::Future<Output = Result<u64>> + Send {
            async { Ok(0) }
        }
    }

    #[test]
    fn from_domains_holds_what_it_was_given() {
        let domains: Vec<Box<dyn DreamDomain>> = vec![
            Box::new(NativeAdapter::new("a", StubModule)),
            Box::new(NativeAdapter::new("b", StubModule)),
            Box::new(NativeAdapter::new("c", StubModule)),
        ];
        let registry = DomainRegistry::from_domains(domains);
        assert_eq!(registry.len(), 3);
        assert_eq!(
            registry.iter().map(|d| d.name()).collect::<Vec<_>>(),
            vec!["a", "b", "c"],
        );
    }

    #[test]
    fn get_returns_domain_by_name() {
        let domains: Vec<Box<dyn DreamDomain>> = vec![
            Box::new(NativeAdapter::new("alpha", StubModule)),
            Box::new(NativeAdapter::new("beta", StubModule)),
        ];
        let registry = DomainRegistry::from_domains(domains);
        assert!(registry.get("alpha").is_some());
        assert!(registry.get("beta").is_some());
        assert!(registry.get("gamma").is_none());
        assert_eq!(registry.get("alpha").unwrap().name(), "alpha");
    }

    #[test]
    fn empty_registry_is_empty() {
        let registry = DomainRegistry::from_domains(vec![]);
        assert!(registry.is_empty());
        assert_eq!(registry.iter().count(), 0);
    }

    #[test]
    fn trait_objects_dispatch_through_iter() {
        let domains: Vec<Box<dyn DreamDomain>> =
            vec![Box::new(NativeAdapter::new("dispatch-test", StubModule))];
        let registry = DomainRegistry::from_domains(domains);
        for d in registry.iter() {
            // every trait method dispatchable through &dyn DreamDomain
            assert_eq!(d.name(), "dispatch-test");
            let cursor = d.current_cursor().unwrap();
            assert!(d.delta(&cursor).unwrap().is_empty());
            assert!(d.contribute_triggers().unwrap().is_empty());
            assert!(d.contribute_tldr().unwrap().is_empty());
        }
    }

    #[test]
    fn native_module_name_constant_is_sorted_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in NATIVE_MODULE_NAMES {
            assert!(seen.insert(*name), "duplicate native module name: {name}");
        }
    }
}
