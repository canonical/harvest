use std::path::Path;

use crate::graph::model::{FunctionNode, ParsedFile};
use super::LanguageParser;

fn relative_path(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

pub struct RustParser;

impl LanguageParser for RustParser {
    fn language_name(&self) -> &str { "rust" }
    fn extensions(&self) -> &[&'static str] { &["rs"] }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_rust::language()).unwrap();

        let tree = match ts_parser.parse(source, None) {
            Some(t) => t,
            None => return ParsedFile::default(),
        };

        let file_path = path.to_string_lossy().into_owned();
        let mut out = ParsedFile {
            path: file_path.clone(),
            language: "rust".into(),
            ..Default::default()
        };

        let fn_query_src = r#"
            (function_item
                name: (identifier) @name
                parameters: (parameters) @params
                return_type: (_)? @ret
            ) @fn
        "#;

        let query = tree_sitter::Query::new(&tree_sitter_rust::language(), fn_query_src)
            .expect("valid Rust function query");
        let mut cursor = tree_sitter::QueryCursor::new();
        let matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        let fn_idx  = query.capture_index_for_name("fn").unwrap();
        let name_idx = query.capture_index_for_name("name").unwrap();

        for m in matches {
            let fn_node   = m.captures.iter().find(|c| c.index == fn_idx).map(|c| c.node);
            let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);

            if let (Some(fn_node), Some(name_node)) = (fn_node, name_node) {
                let name      = &source[name_node.byte_range()];
                let start_row = fn_node.start_position().row as u32 + 1;
                let end_row   = fn_node.end_position().row as u32 + 1;
                let src_text  = &source[fn_node.byte_range()];

                out.functions.push(FunctionNode {
                    repo: repo.into(),
                    version: version.into(),
                    file: file_path.clone(),
                    name: name.into(),
                    signature: first_line(src_text),
                    start_line: start_row,
                    end_line: end_row,
                    source: src_text.into(),
                });
            }
        }

        out
    }
}

pub struct PythonParser;

impl LanguageParser for PythonParser {
    fn language_name(&self) -> &str { "python" }
    fn extensions(&self) -> &[&'static str] { &["py"] }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_python::language()).unwrap();
        ParsedFile {
            path: path.to_string_lossy().into_owned(),
            language: "python".into(),
            ..Default::default()
        }
    }
}

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn language_name(&self) -> &str { "typescript" }
    fn extensions(&self) -> &[&'static str] { &["ts", "tsx"] }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_typescript::language_typescript()).unwrap();
        ParsedFile {
            path: path.to_string_lossy().into_owned(),
            language: "typescript".into(),
            ..Default::default()
        }
    }
}

pub struct JavaScriptParser;

impl LanguageParser for JavaScriptParser {
    fn language_name(&self) -> &str { "javascript" }
    fn extensions(&self) -> &[&'static str] { &["js", "jsx", "mjs"] }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_javascript::language()).unwrap();
        ParsedFile {
            path: path.to_string_lossy().into_owned(),
            language: "javascript".into(),
            ..Default::default()
        }
    }
}

pub struct GoParser;

impl LanguageParser for GoParser {
    fn language_name(&self) -> &str { "go" }
    fn extensions(&self) -> &[&'static str] { &["go"] }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_go::language()).unwrap();
        ParsedFile {
            path: path.to_string_lossy().into_owned(),
            language: "go".into(),
            ..Default::default()
        }
    }
}

pub struct CParser;

impl LanguageParser for CParser {
    fn language_name(&self) -> &str { "c" }
    fn extensions(&self) -> &[&'static str] { &["c", "h"] }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_c::language()).unwrap();
        ParsedFile {
            path: path.to_string_lossy().into_owned(),
            language: "c".into(),
            ..Default::default()
        }
    }
}

pub struct CppParser;

impl LanguageParser for CppParser {
    fn language_name(&self) -> &str { "cpp" }
    fn extensions(&self) -> &[&'static str] { &["cpp", "cc", "cxx", "hpp"] }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_cpp::language()).unwrap();
        ParsedFile {
            path: path.to_string_lossy().into_owned(),
            language: "cpp".into(),
            ..Default::default()
        }
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(text)
        .trim()
        .to_string()
}
