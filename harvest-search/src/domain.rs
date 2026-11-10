use serde::{Deserialize, Serialize};

use crate::trace::Trace;

pub type UnitId = u32;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Scope {
    pub repo: String,
    pub version: String,
}

impl Scope {
    pub fn new(repo: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            version: version.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBudget(pub usize);

impl Default for TokenBudget {
    fn default() -> Self {
        Self(8_000)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub scope: Scope,
    #[serde(default)]
    pub budget: TokenBudget,
}

impl Query {
    pub fn new(text: impl Into<String>, scope: Scope) -> Self {
        Self {
            text: text.into(),
            scope,
            budget: TokenBudget::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceKey {
    Symbol {
        file: String,
        name: String,
    },
    Span {
        file: String,
        start_line: u32,
        end_line: u32,
    },
}

impl EvidenceKey {
    pub fn file(&self) -> &str {
        match self {
            Self::Symbol { file, .. } | Self::Span { file, .. } => file,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Lexical,
    Dense,
    Exact,
    Graph,
    Plan,
    Step,
    Fallback,
    Cache,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub key: EvidenceKey,
    #[serde(default)]
    pub unit: Option<UnitId>,
    pub label: String,
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
    pub score: f32,
    pub found_by: Source,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<EvidenceKey>,
}

impl Evidence {
    pub fn keys(&self) -> impl Iterator<Item = &EvidenceKey> {
        std::iter::once(&self.key).chain(self.covers.iter())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Gap {
    DanglingReference {
        name: String,
        referenced_by: String,
    },
    IncompleteList {
        of: String,
        found: usize,
        total: usize,
    },
    Unanswered {
        hint: String,
    },
}

impl Gap {
    pub fn describe(&self) -> String {
        match self {
            Self::DanglingReference {
                name,
                referenced_by,
            } => format!("{referenced_by} refers to {name}, which is not in the evidence"),
            Self::IncompleteList { of, found, total } => {
                format!("only {found} of {total} {of} are in the evidence")
            }
            Self::Unanswered { hint } => hint.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierId {
    Fallback,
    Rounds,
    Plan,
    Steps,
}

impl TierId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fallback => "fallback",
            Self::Rounds => "tier1",
            Self::Plan => "tier2",
            Self::Steps => "tier3",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Degraded {
    IndexCold,
    ModelUnavailable,
    DeadlineTruncated,
}

impl Degraded {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IndexCold => "index_cold",
            Self::ModelUnavailable => "model_unavailable",
            Self::DeadlineTruncated => "deadline_truncated",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub evidence: Vec<Evidence>,
    pub gaps: Vec<Gap>,
    pub tier_reached: TierId,
    pub degraded: Option<Degraded>,
    pub generation: Option<String>,
    pub trace: Trace,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("the index for {0:?} is not loaded on this node yet")]
    IndexCold(Scope),
    #[error("unknown repository or version {0:?}")]
    UnknownScope(Scope),
    #[error("language model failed: {0}")]
    Model(String),
    #[error("invalid plan: {0}")]
    Plan(String),
    #[error(transparent)]
    Store(#[from] anyhow::Error),
}
