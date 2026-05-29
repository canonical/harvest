use std::path::Path;

use crate::graph::model::{ClassNode, FunctionNode, ImportNode, ParsedFile};
use super::LanguageParser;

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

        let tree = match ts_parser.parse(source, None) {
            Some(t) => t,
            None => return ParsedFile::default(),
        };

        let file_path = path.to_string_lossy().into_owned();
        let lang = tree_sitter_python::language();
        let mut out = ParsedFile {
            path: file_path.clone(),
            language: "python".into(),
            ..Default::default()
        };

        // ── Functions and methods ──────────────────────────────────────────
        let fn_query = tree_sitter::Query::new(
            &lang,
            "(function_definition name: (identifier) @name) @fn",
        ).expect("valid Python function query");
        let fn_idx   = fn_query.capture_index_for_name("fn").unwrap();
        let name_idx = fn_query.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&fn_query, tree.root_node(), source.as_bytes()) {
                let fn_node   = m.captures.iter().find(|c| c.index == fn_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);
                if let (Some(fn_n), Some(nm_n)) = (fn_node, name_node) {
                    let name       = &source[nm_n.byte_range()];
                    let start_line = fn_n.start_position().row as u32 + 1;
                    let end_line   = fn_n.end_position().row as u32 + 1;
                    let src_text   = &source[fn_n.byte_range()];
                    out.functions.push(FunctionNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file_path.clone(),
                        name: name.into(),
                        signature: first_line(src_text),
                        start_line,
                        end_line,
                        source: src_text.into(),
                    });
                }
            }
        }

        // ── Classes ───────────────────────────────────────────────────────
        let cls_query = tree_sitter::Query::new(
            &lang,
            "(class_definition name: (identifier) @name) @class",
        ).expect("valid Python class query");
        let cls_idx      = cls_query.capture_index_for_name("class").unwrap();
        let cls_name_idx = cls_query.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&cls_query, tree.root_node(), source.as_bytes()) {
                let cls_node  = m.captures.iter().find(|c| c.index == cls_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == cls_name_idx).map(|c| c.node);
                if let (Some(cls_n), Some(nm_n)) = (cls_node, name_node) {
                    let name       = &source[nm_n.byte_range()];
                    let start_line = cls_n.start_position().row as u32 + 1;
                    let end_line   = cls_n.end_position().row as u32 + 1;
                    let src_text   = &source[cls_n.byte_range()];
                    out.classes.push(ClassNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file_path.clone(),
                        name: name.into(),
                        start_line,
                        end_line,
                        source: src_text.into(),
                    });
                }
            }
        }

        // ── Imports ───────────────────────────────────────────────────────
        let imp_query = tree_sitter::Query::new(
            &lang,
            "[(import_statement) (import_from_statement)] @import",
        ).expect("valid Python import query");
        let imp_idx = imp_query.capture_index_for_name("import").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&imp_query, tree.root_node(), source.as_bytes()) {
                if let Some(cap) = m.captures.iter().find(|c| c.index == imp_idx) {
                    let node   = cap.node;
                    let line   = node.start_position().row as u32 + 1;
                    let target = python_import_target(source, node);
                    if !target.is_empty() {
                        out.imports.push(ImportNode {
                            repo: repo.into(),
                            version: version.into(),
                            file: file_path.clone(),
                            target,
                            line,
                        });
                    }
                }
            }
        }

        out
    }
}

fn python_import_target(source: &str, node: tree_sitter::Node) -> String {
    match node.kind() {
        "import_from_statement" => node
            .child_by_field_name("module_name")
            .map(|n| source[n.byte_range()].trim().to_string())
            .unwrap_or_default(),
        "import_statement" => {
            for i in 0..node.named_child_count() {
                let child = node.named_child(i).unwrap();
                match child.kind() {
                    "dotted_name" => return source[child.byte_range()].trim().to_string(),
                    "aliased_import" => {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            return source[name_node.byte_range()].trim().to_string();
                        }
                    }
                    _ => {}
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn language_name(&self) -> &str { "typescript" }
    fn extensions(&self) -> &[&'static str] { &["ts", "tsx"] }

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
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

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
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

        let tree = match ts_parser.parse(source, None) {
            Some(t) => t,
            None => return ParsedFile::default(),
        };

        let file_path = path.to_string_lossy().into_owned();
        let lang = tree_sitter_go::language();
        let mut out = ParsedFile {
            path: file_path.clone(),
            language: "go".into(),
            ..Default::default()
        };

        // ── Functions and methods ──────────────────────────────────────────
        let fn_query = tree_sitter::Query::new(
            &lang,
            "[(function_declaration name: (identifier) @name) @fn
              (method_declaration   name: (field_identifier) @name) @fn]",
        ).expect("valid Go function query");
        let fn_idx   = fn_query.capture_index_for_name("fn").unwrap();
        let name_idx = fn_query.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&fn_query, tree.root_node(), source.as_bytes()) {
                let fn_node   = m.captures.iter().find(|c| c.index == fn_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);
                if let (Some(fn_n), Some(nm_n)) = (fn_node, name_node) {
                    let name       = &source[nm_n.byte_range()];
                    let start_line = fn_n.start_position().row as u32 + 1;
                    let end_line   = fn_n.end_position().row as u32 + 1;
                    let src_text   = &source[fn_n.byte_range()];
                    out.functions.push(FunctionNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file_path.clone(),
                        name: name.into(),
                        signature: first_line(src_text),
                        start_line,
                        end_line,
                        source: src_text.into(),
                    });
                }
            }
        }

        // ── Structs and interfaces (mapped to ClassNode) ───────────────────
        let type_query = tree_sitter::Query::new(
            &lang,
            "(type_spec name: (type_identifier) @name) @type_spec",
        ).expect("valid Go type query");
        let ts_idx      = type_query.capture_index_for_name("type_spec").unwrap();
        let ts_name_idx = type_query.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&type_query, tree.root_node(), source.as_bytes()) {
                let ts_node   = m.captures.iter().find(|c| c.index == ts_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == ts_name_idx).map(|c| c.node);
                if let (Some(ts_n), Some(nm_n)) = (ts_node, name_node) {
                    // Only index struct and interface types; skip simple type aliases.
                    let is_composite = ts_n.children(&mut ts_n.walk()).any(|c| {
                        matches!(c.kind(), "struct_type" | "interface_type")
                    });
                    if !is_composite { continue; }
                    let name       = &source[nm_n.byte_range()];
                    let start_line = ts_n.start_position().row as u32 + 1;
                    let end_line   = ts_n.end_position().row as u32 + 1;
                    let src_text   = &source[ts_n.byte_range()];
                    out.classes.push(ClassNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file_path.clone(),
                        name: name.into(),
                        start_line,
                        end_line,
                        source: src_text.into(),
                    });
                }
            }
        }

        // ── Imports ───────────────────────────────────────────────────────
        let imp_query = tree_sitter::Query::new(
            &lang,
            "(import_spec path: (interpreted_string_literal) @path)",
        ).expect("valid Go import query");
        let path_idx = imp_query.capture_index_for_name("path").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&imp_query, tree.root_node(), source.as_bytes()) {
                if let Some(cap) = m.captures.iter().find(|c| c.index == path_idx) {
                    let line   = cap.node.start_position().row as u32 + 1;
                    // Strip surrounding quotes from "fmt" → fmt
                    let raw    = &source[cap.node.byte_range()];
                    let target = raw.trim_matches('"').to_string();
                    if !target.is_empty() {
                        out.imports.push(ImportNode {
                            repo: repo.into(),
                            version: version.into(),
                            file: file_path.clone(),
                            target,
                            line,
                        });
                    }
                }
            }
        }

        out
    }
}

pub struct CParser;

impl LanguageParser for CParser {
    fn language_name(&self) -> &str { "c" }
    fn extensions(&self) -> &[&'static str] { &["c", "h"] }

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
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

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use super::*;
    use crate::parser::LanguageParser;

    fn parse_rust(source: &str) -> crate::graph::model::ParsedFile {
        RustParser.parse(source, Path::new("src/lib.rs"), "myrepo", "v1.0")
    }

    // ── Rust parser ──────────────────────────────────────────────────────────

    #[test]
    fn rust_single_function() {
        let src = r#"fn hello() { println!("hi"); }"#;
        let pf = parse_rust(src);
        assert_eq!(pf.functions.len(), 1);
        let f = &pf.functions[0];
        assert_eq!(f.name, "hello");
        assert_eq!(f.start_line, 1);
        assert_eq!(f.end_line, 1);
        assert!(f.source.contains("println"));
        assert_eq!(f.repo, "myrepo");
        assert_eq!(f.version, "v1.0");
    }

    #[test]
    fn rust_two_functions() {
        let src = "fn foo() {}\nfn bar() {}";
        let pf = parse_rust(src);
        assert_eq!(pf.functions.len(), 2);
        let names: Vec<&str> = pf.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
    }

    #[test]
    fn rust_function_line_numbers() {
        let src = "struct S;\n\nfn third_line() {}";
        let pf = parse_rust(src);
        assert_eq!(pf.functions.len(), 1);
        assert_eq!(pf.functions[0].start_line, 3);
        assert_eq!(pf.functions[0].end_line, 3);
    }

    #[test]
    fn rust_function_with_return_type_signature() {
        let src = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let pf = parse_rust(src);
        assert_eq!(pf.functions.len(), 1);
        let sig = &pf.functions[0].signature;
        assert!(sig.contains("fn add"), "signature was: {sig}");
        assert!(sig.contains("->"), "signature missing return type: {sig}");
    }

    #[test]
    fn rust_struct_only_no_functions() {
        let src = "pub struct Foo { x: i32 }";
        let pf = parse_rust(src);
        assert_eq!(pf.functions.len(), 0);
    }

    #[test]
    fn rust_empty_file() {
        let pf = parse_rust("");
        assert_eq!(pf.functions.len(), 0);
        assert_eq!(pf.classes.len(), 0);
        assert_eq!(pf.imports.len(), 0);
    }

    #[test]
    fn rust_parsed_file_metadata() {
        let pf = parse_rust("fn f() {}");
        assert_eq!(pf.language, "rust");
        assert_eq!(pf.path, "src/lib.rs");
    }

    #[test]
    fn rust_multiline_function_end_line() {
        let src = "fn multi() {\n    let x = 1;\n    let y = 2;\n}";
        let pf = parse_rust(src);
        assert_eq!(pf.functions.len(), 1);
        assert_eq!(pf.functions[0].start_line, 1);
        assert_eq!(pf.functions[0].end_line, 4);
    }

    #[test]
    fn rust_source_text_captured() {
        let src = "fn greet() { \"hello\" }";
        let pf = parse_rust(src);
        assert_eq!(pf.functions[0].source, src);
    }

    // ── Python parser ─────────────────────────────────────────────────────────

    fn parse_python(source: &str) -> ParsedFile {
        PythonParser.parse(source, Path::new("module.py"), "myrepo", "v1.0")
    }

    #[test]
    fn python_single_function() {
        let pf = parse_python("def hello():\n    pass");
        assert_eq!(pf.functions.len(), 1);
        assert_eq!(pf.functions[0].name, "hello");
        assert_eq!(pf.functions[0].start_line, 1);
        assert_eq!(pf.functions[0].end_line, 2);
    }

    #[test]
    fn python_function_with_args() {
        let pf = parse_python("def greet(name, greeting='Hi'):\n    return greeting + name");
        assert_eq!(pf.functions.len(), 1);
        assert_eq!(pf.functions[0].name, "greet");
    }

    #[test]
    fn python_two_functions() {
        let pf = parse_python("def foo():\n    pass\n\ndef bar():\n    pass");
        assert_eq!(pf.functions.len(), 2);
        let names: Vec<&str> = pf.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"foo") && names.contains(&"bar"));
    }

    #[test]
    fn python_class_definition() {
        let pf = parse_python("class Foo:\n    pass");
        assert_eq!(pf.classes.len(), 1);
        assert_eq!(pf.classes[0].name, "Foo");
        assert_eq!(pf.classes[0].start_line, 1);
    }

    #[test]
    fn python_class_with_method() {
        let src = "class Foo:\n    def bar(self):\n        pass";
        let pf = parse_python(src);
        assert_eq!(pf.classes.len(), 1);
        assert_eq!(pf.classes[0].name, "Foo");
        assert_eq!(pf.functions.len(), 1);
        assert_eq!(pf.functions[0].name, "bar");
    }

    #[test]
    fn python_two_classes() {
        let pf = parse_python("class A:\n    pass\n\nclass B:\n    pass");
        assert_eq!(pf.classes.len(), 2);
        let names: Vec<&str> = pf.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"A") && names.contains(&"B"));
    }

    #[test]
    fn python_simple_import() {
        let pf = parse_python("import os");
        assert_eq!(pf.imports.len(), 1);
        assert_eq!(pf.imports[0].target, "os");
        assert_eq!(pf.imports[0].line, 1);
    }

    #[test]
    fn python_dotted_import() {
        let pf = parse_python("import os.path");
        assert_eq!(pf.imports.len(), 1);
        assert_eq!(pf.imports[0].target, "os.path");
    }

    #[test]
    fn python_aliased_import() {
        let pf = parse_python("import numpy as np");
        assert_eq!(pf.imports.len(), 1);
        assert_eq!(pf.imports[0].target, "numpy");
    }

    #[test]
    fn python_from_import() {
        let pf = parse_python("from os import path");
        assert_eq!(pf.imports.len(), 1);
        assert_eq!(pf.imports[0].target, "os");
    }

    #[test]
    fn python_from_dotted_import() {
        let pf = parse_python("from sunbeam.utils import helpers");
        assert_eq!(pf.imports.len(), 1);
        assert_eq!(pf.imports[0].target, "sunbeam.utils");
    }

    #[test]
    fn python_source_text_captured() {
        let src = "def greet():\n    return 'hi'";
        let pf = parse_python(src);
        assert!(pf.functions[0].source.contains("return 'hi'"));
    }

    #[test]
    fn python_class_source_text_captured() {
        let src = "class Foo:\n    x = 1";
        let pf = parse_python(src);
        assert!(pf.classes[0].source.contains("x = 1"));
    }

    #[test]
    fn python_empty_file() {
        let pf = parse_python("");
        assert!(pf.functions.is_empty());
        assert!(pf.classes.is_empty());
        assert!(pf.imports.is_empty());
        assert_eq!(pf.language, "python");
    }

    #[test]
    fn python_parsed_file_metadata() {
        let pf = parse_python("def f():\n    pass");
        assert_eq!(pf.language, "python");
        assert_eq!(pf.path, "module.py");
        assert_eq!(pf.functions[0].repo, "myrepo");
        assert_eq!(pf.functions[0].version, "v1.0");
    }

    // ── Go parser ─────────────────────────────────────────────────────────────

    fn parse_go(source: &str) -> ParsedFile {
        GoParser.parse(source, Path::new("main.go"), "myrepo", "v1.0")
    }

    #[test]
    fn go_single_function() {
        let pf = parse_go("package main\n\nfunc Hello() {}");
        assert_eq!(pf.functions.len(), 1);
        assert_eq!(pf.functions[0].name, "Hello");
    }

    #[test]
    fn go_two_functions() {
        let pf = parse_go("package main\n\nfunc Foo() {}\n\nfunc Bar() {}");
        assert_eq!(pf.functions.len(), 2);
        let names: Vec<&str> = pf.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"Foo") && names.contains(&"Bar"));
    }

    #[test]
    fn go_method() {
        let src = "package main\n\ntype Foo struct{}\n\nfunc (f Foo) Bar() {}";
        let pf = parse_go(src);
        assert!(pf.functions.iter().any(|f| f.name == "Bar"));
    }

    #[test]
    fn go_struct_as_class() {
        let src = "package main\n\ntype Foo struct {\n    X int\n}";
        let pf = parse_go(src);
        assert_eq!(pf.classes.len(), 1);
        assert_eq!(pf.classes[0].name, "Foo");
    }

    #[test]
    fn go_single_import() {
        let src = "package main\nimport \"fmt\"\nfunc main() {}";
        let pf = parse_go(src);
        assert!(pf.imports.iter().any(|i| i.target == "fmt"));
    }

    #[test]
    fn go_grouped_imports() {
        let src = "package main\nimport (\n    \"fmt\"\n    \"os\"\n)\nfunc main() {}";
        let pf = parse_go(src);
        let targets: Vec<&str> = pf.imports.iter().map(|i| i.target.as_str()).collect();
        assert!(targets.contains(&"fmt") && targets.contains(&"os"));
    }

    #[test]
    fn go_empty_file() {
        let pf = parse_go("package main");
        assert!(pf.functions.is_empty());
        assert_eq!(pf.language, "go");
    }

    // ── Remaining stubs ───────────────────────────────────────────────────────

    #[test]
    fn typescript_stub_returns_empty() {
        let pf = TypeScriptParser.parse("function foo() {}", Path::new("a.ts"), "r", "v1");
        assert!(pf.functions.is_empty(), "TypeScript parser is still a stub");
        assert_eq!(pf.language, "typescript");
    }

    #[test]
    fn javascript_stub_returns_empty() {
        let pf = JavaScriptParser.parse("function foo() {}", Path::new("a.js"), "r", "v1");
        assert!(pf.functions.is_empty(), "JavaScript parser is still a stub");
        assert_eq!(pf.language, "javascript");
    }

    #[test]
    fn c_stub_returns_empty() {
        let pf = CParser.parse("void foo() {}", Path::new("a.c"), "r", "v1");
        assert!(pf.functions.is_empty(), "C parser is still a stub");
        assert_eq!(pf.language, "c");
    }

    #[test]
    fn cpp_stub_returns_empty() {
        let pf = CppParser.parse("void foo() {}", Path::new("a.cpp"), "r", "v1");
        assert!(pf.functions.is_empty(), "C++ parser is still a stub");
        assert_eq!(pf.language, "cpp");
    }
}
