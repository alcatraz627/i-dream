//! Three-layer consolidation pipeline — L1 per-domain, L2 daily roll-up,
//! L3 weekly audit. Full design at `docs/16-consolidation-build.md`.
//!
//! Stage 2 (this commit) ships **deterministic** L2: the daily digest file
//! exists every day, even before LLM enrichment lands in Stage 3. Sections
//! that need LLM input (Top signals, Cross-domain associations) carry a
//! placeholder until then.

pub mod l2_digest;
