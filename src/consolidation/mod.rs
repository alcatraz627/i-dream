//! Consolidation: what happens to stored learning between sessions.
//! Reinforcement and forgetting shape the pattern store; views and schemas
//! give it honest read models. The cross-stream reader lives in `reader`.

pub mod assay;
pub mod autonomous;
pub mod forgetting;
pub mod reinforce;
pub mod schemas;
pub mod views;
pub mod yield_slo;
