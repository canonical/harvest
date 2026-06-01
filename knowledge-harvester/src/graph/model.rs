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
    pub kind: String,
    pub signature: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source: String,
    pub impl_type: Option<String>,
    pub calls: Vec<CallRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassNode {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source: String,
    pub bases: Vec<String>,
    pub traits: Vec<String>,
    pub embeds: Vec<String>,
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
