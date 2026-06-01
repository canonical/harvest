use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryNode {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionNode {
    pub repo: String,
    pub tag: String,
    pub commit_sha: String,
    pub timestamp: i64,
    pub ingested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub repo: String,
    pub version: String,
    pub path: String,
    pub language: String,
}

/// A single outgoing function call recorded during parsing.
/// Stored as a list on the Function node so `link_call_edges` can resolve them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRef {
    pub callee: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionNode {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub name: String,
    /// Language-specific kind: "function" or "method".
    pub kind: String,
    pub signature: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source: String,
    /// For methods: the name of the owning type (struct / class / interface).
    /// Used to build `contains` edges for Go and Rust where methods are
    /// defined outside the type's line range.
    pub impl_type: Option<String>,
    /// Outgoing calls recorded at parse time; resolved into CALLS edges by the writer.
    pub calls: Vec<CallRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassNode {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub name: String,
    /// Language-specific kind: "class", "struct", "enum", "trait", "interface".
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source: String,
    /// Python / C++: names of base classes (creates INHERITS edges).
    pub bases: Vec<String>,
    /// Rust: names of traits this type explicitly implements (creates IMPLEMENTS edges).
    pub traits: Vec<String>,
    /// Go: names of types anonymously embedded in this struct (creates EMBEDS edges).
    pub embeds: Vec<String>,
    /// Rust: names of user-defined types referenced in field declarations (creates USES edges).
    pub uses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportNode {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub target: String,
    pub line: u32,
}

#[derive(Debug, Default, Clone)]
pub struct ParsedFile {
    pub path: String,
    pub language: String,
    pub functions: Vec<FunctionNode>,
    pub classes: Vec<ClassNode>,
    pub imports: Vec<ImportNode>,
}
