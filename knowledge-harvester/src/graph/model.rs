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
pub struct FunctionNode {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub name: String,
    pub signature: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassNode {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportNode {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub target: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub caller_name: String,
    pub callee_name: String,
    pub line: u32,
}

#[derive(Debug, Default, Clone)]
pub struct ParsedFile {
    pub path: String,
    pub language: String,
    pub functions: Vec<FunctionNode>,
    pub classes: Vec<ClassNode>,
    pub imports: Vec<ImportNode>,
    pub calls: Vec<CallEdge>,
}
