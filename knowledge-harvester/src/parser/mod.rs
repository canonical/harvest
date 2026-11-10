pub mod ecmascript;
pub mod generic;
pub mod language;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::graph::model::ParsedFile;

pub const PARSER_VERSION: i32 = 4;

pub trait LanguageParser: Send + Sync {
    fn language_name(&self) -> &str;
    fn extensions(&self) -> &[&'static str];

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile;
}

pub struct ParserRegistry {
    by_extension: HashMap<String, Arc<dyn LanguageParser>>,
}

impl ParserRegistry {
    pub fn with_defaults() -> Self {
        use language::*;

        let mut entries: Vec<Arc<dyn LanguageParser>> = vec![
            Arc::new(RustParser),
            Arc::new(PythonParser),
            Arc::new(TypeScriptParser),
            Arc::new(JavaScriptParser),
            Arc::new(GoParser),
        ];
        entries.extend(generic::parsers());

        let mut by_extension = HashMap::new();
        for parser in entries {
            for ext in parser.extensions() {
                by_extension.insert(ext.to_string(), Arc::clone(&parser));
            }
        }
        Self { by_extension }
    }

    pub fn get(&self, extension: &str) -> Option<&dyn LanguageParser> {
        self.by_extension
            .get(extension)
            .or_else(|| self.by_extension.get(&extension.to_ascii_lowercase()))
            .map(|p| p.as_ref())
    }

    pub fn languages(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .by_extension
            .values()
            .map(|p| p.language_name().to_string())
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

const TEST_MARKERS: &[&str] = &[
    "test",
    "tests",
    "fact",
    "theory",
    "testmethod",
    "testcase",
    "parameterizedtest",
    "repeatedtest",
    "testfactory",
    "testtemplate",
    "pytest",
];

const TEST_SUFFIXES: &[&str] = &["Test", "Tests", "Spec", "TestCase", "IT"];

fn marker_tokens(marker: &str) -> impl Iterator<Item = String> + '_ {
    marker
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase)
}

pub fn is_test_marker(marker: &str) -> bool {
    marker_tokens(marker).any(|t| TEST_MARKERS.contains(&t.as_str()))
}

pub fn is_test_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("test_") || lower.ends_with("_test") || lower.ends_with("_tests") {
        return true;
    }
    let after_prefix = |prefix: &str| {
        name.strip_prefix(prefix)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
    };
    if after_prefix("Test") || after_prefix("test") {
        return true;
    }
    TEST_SUFFIXES.iter().any(|suffix| {
        name.len() > suffix.len()
            && name.ends_with(suffix)
            && name[..name.len() - suffix.len()]
                .chars()
                .last()
                .is_some_and(|c| c.is_lowercase() || c.is_ascii_digit())
    })
}

fn is_marker_kind(kind: &str) -> bool {
    ["attribute", "decorator", "annotation", "pragma", "marker"]
        .iter()
        .any(|w| kind.contains(w))
}

fn is_modifier_kind(kind: &str) -> bool {
    kind.contains("modifier") || is_marker_kind(kind)
}

pub fn markers(source: &str, node: tree_sitter::Node) -> Vec<String> {
    let mut found = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        let kind = sibling.kind();
        if is_marker_kind(kind) {
            found.push(source[sibling.byte_range()].to_string());
        } else if !kind.contains("comment") {
            break;
        }
        previous = sibling.prev_named_sibling();
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if is_modifier_kind(child.kind()) {
            found.push(source[child.byte_range()].to_string());
        }
    }
    if let Some(parent) = node.parent().filter(|p| p.kind().contains("decorated")) {
        let mut cursor = parent.walk();
        for child in parent.named_children(&mut cursor) {
            if is_marker_kind(child.kind()) {
                found.push(source[child.byte_range()].to_string());
            }
        }
    }
    found
}

fn is_scope_kind(kind: &str) -> bool {
    [
        "mod",
        "module",
        "class",
        "namespace",
        "impl",
        "object",
        "describe",
        "suite",
    ]
    .iter()
    .any(|w| kind.split('_').any(|part| part == *w))
        || kind == "declaration_list"
}

fn scope_name(source: &str, node: tree_sitter::Node) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| source[n.byte_range()].to_string())
}

pub fn is_test_node(source: &str, node: tree_sitter::Node, name: &str) -> bool {
    if is_test_name(name) || markers(source, node).iter().any(|m| is_test_marker(m)) {
        return true;
    }
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if is_scope_kind(ancestor.kind()) {
            if markers(source, ancestor).iter().any(|m| is_test_marker(m)) {
                return true;
            }
            if scope_name(source, ancestor).is_some_and(|n| is_test_name(&n)) {
                return true;
            }
        }
        current = ancestor.parent();
    }
    false
}

fn is_block_kind(kind: &str) -> bool {
    [
        "block",
        "body",
        "statements",
        "compound",
        "suite",
        "do_group",
        "brace_group",
    ]
    .iter()
    .any(|w| kind.contains(w))
}

pub fn statement_lines(node: tree_sitter::Node) -> Vec<u32> {
    let mut body = node.child_by_field_name("body").or_else(|| {
        let mut cursor = node.walk();
        let last = node
            .named_children(&mut cursor)
            .filter(|c| is_block_kind(c.kind()))
            .last();
        last
    });
    while let Some(current) = body {
        if current.named_child_count() == 1 {
            let only = current.named_child(0).filter(|c| is_block_kind(c.kind()));
            if only.is_some() {
                body = only;
                continue;
            }
        }
        break;
    }
    let Some(body) = body else {
        return Vec::new();
    };
    let mut cursor = body.walk();
    let mut lines: Vec<u32> = body
        .named_children(&mut cursor)
        .filter(|c| !c.kind().contains("comment"))
        .map(|c| c.start_position().row as u32 + 1)
        .collect();
    lines.dedup();
    lines
}
