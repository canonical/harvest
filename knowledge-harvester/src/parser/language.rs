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

    // ── Stub parsers return empty ParsedFile ─────────────────────────────────

    #[test]
    fn python_stub_returns_empty() {
        let pf = PythonParser.parse("def foo(): pass", Path::new("a.py"), "r", "v1");
        assert!(pf.functions.is_empty(), "Python parser is still a stub");
        assert_eq!(pf.language, "python");
    }

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
    fn go_stub_returns_empty() {
        let pf = GoParser.parse("func foo() {}", Path::new("a.go"), "r", "v1");
        assert!(pf.functions.is_empty(), "Go parser is still a stub");
        assert_eq!(pf.language, "go");
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
