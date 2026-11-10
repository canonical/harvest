use std::path::{Component, Path, PathBuf};

use tree_sitter::{Node, Query, QueryCursor};

use crate::graph::model::{CallRef, ClassNode, FunctionNode, ImportNode, ParsedFile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    JavaScript,
    TypeScript,
    Tsx,
}

impl Dialect {
    pub fn for_path(path: &Path, typescript: bool) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("tsx") => Self::Tsx,
            _ if typescript => Self::TypeScript,
            _ => Self::JavaScript,
        }
    }

    fn language(self) -> tree_sitter::Language {
        match self {
            Self::JavaScript => tree_sitter_javascript::language(),
            Self::TypeScript => tree_sitter_typescript::language_typescript(),
            Self::Tsx => tree_sitter_typescript::language_tsx(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::JavaScript => "javascript",
            Self::TypeScript | Self::Tsx => "typescript",
        }
    }
}

const CALLS: &str = r#"
[(call_expression function: (identifier) @callee)
 (call_expression function: (member_expression property: (property_identifier) @callee))
 (new_expression constructor: (identifier) @callee)
 (new_expression constructor: (member_expression property: (property_identifier) @callee))]
"#;

fn text<'a>(source: &'a str, node: Node) -> &'a str {
    &source[node.byte_range()]
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('{')
        .trim()
        .to_string()
}

fn doc_comment(source: &str, node: Node) -> Option<String> {
    let mut target = node;
    while let Some(parent) = target.parent() {
        if matches!(
            parent.kind(),
            "export_statement" | "lexical_declaration" | "variable_declaration"
        ) {
            target = parent;
        } else {
            break;
        }
    }
    let previous = target.prev_named_sibling()?;
    if previous.kind() != "comment" || previous.end_position().row + 1 < target.start_position().row
    {
        return None;
    }
    let raw = text(source, previous);
    if !raw.starts_with("/**") {
        return None;
    }
    let cleaned: Vec<&str> = raw
        .trim_start_matches("/**")
        .trim_end_matches("*/")
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .collect();
    (!cleaned.is_empty()).then(|| cleaned.join("\n"))
}

fn enclosing_class(source: &str, node: Node) -> Option<String> {
    let mut current = node.parent();
    while let Some(n) = current {
        if matches!(
            n.kind(),
            "class_declaration" | "class" | "abstract_class_declaration"
        ) {
            return n
                .child_by_field_name("name")
                .map(|c| text(source, c).to_string());
        }
        if matches!(
            n.kind(),
            "function_declaration" | "arrow_function" | "function_expression"
        ) {
            return None;
        }
        current = n.parent();
    }
    None
}

fn calls_in(source: &str, node: Node, query: &Query) -> Vec<CallRef> {
    let mut cursor = QueryCursor::new();
    let mut calls = Vec::new();
    for m in cursor.matches(query, node, source.as_bytes()) {
        for capture in m.captures {
            calls.push(CallRef {
                callee: text(source, capture.node).to_string(),
                line: capture.node.start_position().row as u32 + 1,
            });
        }
    }
    calls
}

pub fn resolve_import(file: &str, specifier: &str) -> String {
    if !specifier.starts_with('.') {
        return specifier.to_string();
    }
    let base = Path::new(file).parent().unwrap_or(Path::new(""));
    let mut parts: Vec<String> = Vec::new();
    for component in base.join(specifier).components() {
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    let joined: PathBuf = parts.iter().collect();
    let mut resolved = joined.to_string_lossy().replace('\\', "/");
    for extension in [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
        if let Some(stripped) = resolved.strip_suffix(extension) {
            resolved = stripped.to_string();
            break;
        }
    }
    resolved
}

fn heritage(source: &str, class: Node) -> (Vec<String>, Vec<String>) {
    let mut bases = Vec::new();
    let mut traits = Vec::new();
    let mut walk = class.walk();
    for child in class.named_children(&mut walk) {
        if child.kind() != "class_heritage" {
            continue;
        }
        let mut inner = child.walk();
        let clauses: Vec<Node> = child.named_children(&mut inner).collect();
        if clauses.is_empty() {
            let raw = text(source, child).trim_start_matches("extends").trim();
            if !raw.is_empty() {
                bases.push(raw.split(['(', '<', ' ']).next().unwrap_or(raw).to_string());
            }
        }
        for clause in clauses {
            let target = match clause.kind() {
                "extends_clause" => &mut bases,
                "implements_clause" => &mut traits,
                "identifier" | "member_expression" => {
                    bases.push(text(source, clause).to_string());
                    continue;
                }
                _ => continue,
            };
            let mut values = clause.walk();
            for value in clause.named_children(&mut values) {
                if matches!(
                    value.kind(),
                    "identifier"
                        | "type_identifier"
                        | "member_expression"
                        | "nested_type_identifier"
                        | "generic_type"
                ) {
                    let name = text(source, value);
                    target.push(name.split('<').next().unwrap_or(name).to_string());
                }
            }
        }
    }
    (bases, traits)
}

fn decorators(source: &str, class: Node) -> Vec<String> {
    let mut found = Vec::new();
    let mut walk = class.walk();
    for child in class.children(&mut walk) {
        if child.kind() == "decorator" {
            let raw = text(source, child).trim_start_matches('@');
            found.push(raw.split('(').next().unwrap_or(raw).trim().to_string());
        }
    }
    if let Some(parent) = class.parent().filter(|p| p.kind() == "export_statement") {
        let mut walk = parent.walk();
        for child in parent.children(&mut walk) {
            if child.kind() == "decorator" {
                let raw = text(source, child).trim_start_matches('@');
                found.push(raw.split('(').next().unwrap_or(raw).trim().to_string());
            }
        }
    }
    found
}

pub fn parse(source: &str, path: &Path, repo: &str, version: &str, dialect: Dialect) -> ParsedFile {
    let language = dialect.language();
    let mut parser = tree_sitter::Parser::new();
    let file = path.to_string_lossy().replace('\\', "/");
    let mut out = ParsedFile {
        path: file.clone(),
        language: dialect.name().into(),
        ..ParsedFile::default()
    };
    if parser.set_language(&language).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else {
        return out;
    };
    let root = tree.root_node();
    let calls = Query::new(&language, CALLS).expect("valid call query");

    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let mut walk = node.walk();
        for child in node.named_children(&mut walk) {
            stack.push(child);
        }
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        match node.kind() {
            "function_declaration" | "generator_function_declaration" | "method_definition" => {
                let Some(name) = node.child_by_field_name("name") else {
                    continue;
                };
                let impl_type = (node.kind() == "method_definition")
                    .then(|| enclosing_class(source, node))
                    .flatten();
                let body = text(source, node);
                out.functions.push(FunctionNode {
                    repo: repo.into(),
                    version: version.into(),
                    file: file.clone(),
                    name: text(source, name).into(),
                    kind: if impl_type.is_some() {
                        "method"
                    } else {
                        "function"
                    }
                    .into(),
                    signature: first_line(body),
                    start_line,
                    end_line,
                    source: body.into(),
                    impl_type,
                    docstring: doc_comment(source, node),
                    calls: calls_in(source, node, &calls),
                });
            }
            "variable_declarator" => {
                let (Some(name), Some(value)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ) else {
                    continue;
                };
                if !matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "function"
                ) {
                    continue;
                }
                let declaration = node.parent().unwrap_or(node);
                let body = text(source, declaration);
                out.functions.push(FunctionNode {
                    repo: repo.into(),
                    version: version.into(),
                    file: file.clone(),
                    name: text(source, name).into(),
                    kind: "function".into(),
                    signature: first_line(body),
                    start_line: declaration.start_position().row as u32 + 1,
                    end_line: declaration.end_position().row as u32 + 1,
                    source: body.into(),
                    impl_type: None,
                    docstring: doc_comment(source, declaration),
                    calls: calls_in(source, value, &calls),
                });
            }
            "class_declaration" | "abstract_class_declaration" | "interface_declaration" => {
                let Some(name) = node.child_by_field_name("name") else {
                    continue;
                };
                let (mut bases, traits) = heritage(source, node);
                if node.kind() == "interface_declaration" {
                    let mut walk = node.walk();
                    for child in node.named_children(&mut walk) {
                        if child.kind() == "extends_type_clause" {
                            let mut inner = child.walk();
                            for t in child.named_children(&mut inner) {
                                bases.push(
                                    text(source, t).split('<').next().unwrap_or("").to_string(),
                                );
                            }
                        }
                    }
                }
                out.classes.push(ClassNode {
                    repo: repo.into(),
                    version: version.into(),
                    file: file.clone(),
                    name: text(source, name).into(),
                    kind: if node.kind() == "interface_declaration" {
                        "interface"
                    } else {
                        "class"
                    }
                    .into(),
                    start_line,
                    end_line,
                    source: text(source, node).into(),
                    bases,
                    traits,
                    embeds: vec![],
                    uses: vec![],
                    docstring: doc_comment(source, node),
                    decorators: decorators(source, node),
                });
            }
            "import_statement" => {
                if let Some(specifier) = node.child_by_field_name("source") {
                    let raw = text(source, specifier).trim_matches(['"', '\'', '`']);
                    out.imports.push(ImportNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file.clone(),
                        target: resolve_import(&file, raw),
                        line: start_line,
                    });
                }
            }
            "call_expression" => {
                let is_require = node
                    .child_by_field_name("function")
                    .is_some_and(|f| text(source, f) == "require");
                if !is_require {
                    continue;
                }
                let argument = node
                    .child_by_field_name("arguments")
                    .and_then(|a| a.named_child(0))
                    .filter(|a| a.kind() == "string");
                if let Some(argument) = argument {
                    let raw = text(source, argument).trim_matches(['"', '\'', '`']);
                    out.imports.push(ImportNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file.clone(),
                        target: resolve_import(&file, raw),
                        line: start_line,
                    });
                }
            }
            _ => {}
        }
    }
    out.functions.sort_by_key(|f| f.start_line);
    out.classes.sort_by_key(|c| c.start_line);
    out.imports.sort_by_key(|i| i.line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: &str = r#"
import { Base } from "./base";
import * as fs from "fs";

/** Stores volumes on disk. */
export class DiskStore extends Base implements Store, Closeable {
  /** Write a volume. */
  write(volume: Volume): void {
    this.flush();
    const id = new VolumeId(volume.name);
    fs.writeFileSync(id.path(), "x");
  }

  private flush() {}
}

export interface Store extends Closeable<Volume> {
  write(volume: Volume): void;
}

export const helper = (x: number) => format(x);

function standalone() {
  return helper(1);
}
"#;

    #[test]
    fn typescript_classes_methods_functions_and_imports_are_extracted() {
        let parsed = parse(
            TS,
            Path::new("src/store/disk.ts"),
            "r",
            "v",
            Dialect::TypeScript,
        );
        let class = parsed
            .classes
            .iter()
            .find(|c| c.name == "DiskStore")
            .unwrap();
        assert_eq!(class.bases, vec!["Base"]);
        assert_eq!(class.traits, vec!["Store", "Closeable"]);
        assert_eq!(class.docstring.as_deref(), Some("Stores volumes on disk."));
        let store = parsed.classes.iter().find(|c| c.name == "Store").unwrap();
        assert_eq!(store.kind, "interface");
        assert_eq!(store.bases, vec!["Closeable"]);

        let write = parsed.functions.iter().find(|f| f.name == "write").unwrap();
        assert_eq!(write.impl_type.as_deref(), Some("DiskStore"));
        assert_eq!(write.docstring.as_deref(), Some("Write a volume."));
        let callees: Vec<&str> = write.calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(
            callees.contains(&"flush")
                && callees.contains(&"VolumeId")
                && callees.contains(&"writeFileSync")
        );

        let helper = parsed
            .functions
            .iter()
            .find(|f| f.name == "helper")
            .unwrap();
        assert!(helper.impl_type.is_none());
        assert_eq!(helper.calls[0].callee, "format");
        assert!(parsed.functions.iter().any(|f| f.name == "standalone"));
        assert_eq!(parsed.imports[0].target, "src/store/base");
        assert_eq!(parsed.imports[1].target, "fs");
        assert_eq!(parsed.language, "typescript");
    }

    #[test]
    fn javascript_classes_and_requires_are_extracted() {
        let source = "const util = require('../util');\nclass Cache extends Map {\n  get(k) { return util.hash(k); }\n}\nfunction make() { return new Cache(); }\n";
        let parsed = parse(
            source,
            Path::new("lib/cache.js"),
            "r",
            "v",
            Dialect::JavaScript,
        );
        assert_eq!(parsed.classes[0].name, "Cache");
        assert_eq!(parsed.classes[0].bases, vec!["Map"]);
        assert_eq!(
            parsed
                .functions
                .iter()
                .find(|f| f.name == "get")
                .unwrap()
                .impl_type
                .as_deref(),
            Some("Cache")
        );
        assert_eq!(parsed.imports[0].target, "util");
        assert_eq!(parsed.language, "javascript");
    }

    #[test]
    fn relative_imports_resolve_against_the_importing_file() {
        assert_eq!(resolve_import("src/a/b.ts", "../c/d.js"), "src/c/d");
        assert_eq!(resolve_import("src/a/b.ts", "react"), "react");
    }
}
