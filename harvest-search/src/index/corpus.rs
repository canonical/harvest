use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{EvidenceKey, Scope, UnitId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Function,
    Method,
    Class,
    Module,
    Config,
    Doc,
    Chunk,
}

impl UnitKind {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "function" => Self::Function,
            "method" => Self::Method,
            "class" => Self::Class,
            "module" => Self::Module,
            "config" => Self::Config,
            "doc" => Self::Doc,
            "chunk" => Self::Chunk,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Module => "module",
            Self::Config => "config",
            Self::Doc => "doc",
            Self::Chunk => "chunk",
        }
    }

    pub fn is_symbol(self) -> bool {
        matches!(self, Self::Function | Self::Method | Self::Class)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    pub id: UnitId,
    pub db_id: i64,
    pub kind: Option<UnitKind>,
    pub file: String,
    pub name: String,
    pub qualified_name: String,
    pub parent: Option<String>,
    pub signature: String,
    pub docstring: String,
    pub body: String,
    pub start_line: u32,
    pub end_line: u32,
    pub decorators: Vec<String>,
    pub bases: Vec<String>,
    pub language: String,
    #[serde(default)]
    pub summary: String,
}

impl Unit {
    pub fn kind(&self) -> UnitKind {
        self.kind.unwrap_or(UnitKind::Function)
    }

    pub fn key(&self) -> EvidenceKey {
        if self.kind().is_symbol() {
            EvidenceKey::Symbol {
                file: self.file.clone(),
                name: self.qualified_name.clone(),
            }
        } else {
            EvidenceKey::Span {
                file: self.file.clone(),
                start_line: self.start_line,
                end_line: self.end_line,
            }
        }
    }

    pub fn label(&self) -> String {
        format!(
            "{} {} ({})",
            self.kind().as_str(),
            self.qualified_name,
            self.file
        )
    }

    pub fn contains(&self, other: &Unit) -> bool {
        self.id != other.id
            && self.file == other.file
            && self.start_line <= other.start_line
            && self.end_line >= other.end_line
            && (self.start_line, self.end_line) != (other.start_line, other.end_line)
    }

    pub fn is_test(&self) -> bool {
        let file = self.file.to_ascii_lowercase();
        file.contains("/tests/")
            || file.starts_with("tests/")
            || file.contains("/test/")
            || file.contains("_test.")
            || file.contains("/test_")
            || file.starts_with("test_")
            || file.contains(".spec.")
            || file.contains(".test.")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Calls,
    Inherits,
    Implements,
    Uses,
    Embeds,
    Overrides,
    Registers,
    ReadsConfig,
    Instantiates,
    Raises,
    Tests,
    Documents,
    CoChanges,
    Contains,
}

impl Relation {
    pub const ALL: [Relation; 14] = [
        Self::Calls,
        Self::Inherits,
        Self::Implements,
        Self::Uses,
        Self::Embeds,
        Self::Overrides,
        Self::Registers,
        Self::ReadsConfig,
        Self::Instantiates,
        Self::Raises,
        Self::Tests,
        Self::Documents,
        Self::CoChanges,
        Self::Contains,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str().eq_ignore_ascii_case(value))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Calls => "CALLS",
            Self::Inherits => "INHERITS",
            Self::Implements => "IMPLEMENTS",
            Self::Uses => "USES",
            Self::Embeds => "EMBEDS",
            Self::Overrides => "OVERRIDES",
            Self::Registers => "REGISTERS",
            Self::ReadsConfig => "READS_CONFIG",
            Self::Instantiates => "INSTANTIATES",
            Self::Raises => "RAISES",
            Self::Tests => "TESTS",
            Self::Documents => "DOCUMENTS",
            Self::CoChanges => "CO_CHANGES",
            Self::Contains => "CONTAINS",
        }
    }

    pub fn is_hierarchy(self) -> bool {
        matches!(self, Self::Inherits | Self::Implements)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Edge {
    pub src: UnitId,
    pub dst: UnitId,
    pub relation: Relation,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoredVectors {
    pub model: String,
    pub vectors: Vec<Option<Vec<f32>>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Corpus {
    pub scope: Scope,
    pub generation: String,
    pub units: Vec<Unit>,
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub vectors: Option<StoredVectors>,
}

impl Corpus {
    pub fn new(scope: Scope, generation: impl Into<String>) -> Self {
        Self {
            scope,
            generation: generation.into(),
            ..Self::default()
        }
    }

    pub fn push_unit(&mut self, mut unit: Unit) -> UnitId {
        let id = self.units.len() as UnitId;
        unit.id = id;
        if unit.qualified_name.is_empty() {
            unit.qualified_name = unit.name.clone();
        }
        self.units.push(unit);
        id
    }

    pub fn link(&mut self, src: UnitId, dst: UnitId, relation: Relation) {
        if src != dst {
            self.edges.push(Edge { src, dst, relation });
        }
    }
}

#[async_trait]
pub trait CorpusSource: Send + Sync {
    async fn generation(&self, scope: &Scope) -> Result<Option<String>>;
    async fn load(&self, scope: &Scope) -> Result<Corpus>;
}
