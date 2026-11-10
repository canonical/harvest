pub mod calibration;
pub mod context;
pub mod deadline;
pub mod domain;
pub mod engine;
pub mod fallback;
pub mod gate;
pub mod index;
pub mod llm;
pub mod question;
pub mod retrieve;
pub mod tiers;
pub mod trace;

pub use deadline::Deadline;
pub use domain::{
    Degraded, Evidence, EvidenceKey, Gap, Query, Scope, SearchError, SearchResult, Source, TierId,
    TokenBudget,
};
pub use engine::{SearchEngine, TieredEngine, WithFallback};
pub use trace::Trace;
