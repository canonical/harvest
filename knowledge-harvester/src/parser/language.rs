use std::collections::HashMap;
use std::path::Path;

use crate::graph::model::{CallRef, ClassNode, FunctionNode, ImportNode, ParsedFile};
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
        let lang = tree_sitter_rust::language();
        let mut out = ParsedFile {
            path: file_path.clone(),
            language: "rust".into(),
            ..Default::default()
        };

        // ── impl_type map: fn start_byte → implementing type name ────────────
        // Covers plain `impl Foo` and generic `impl<T> Foo<T>`.
        let mut impl_type_map: HashMap<usize, String> = HashMap::new();
        for impl_q_src in [
            "(impl_item type: (type_identifier) @impl_type body: (declaration_list (function_item) @fn_in_impl))",
            "(impl_item type: (generic_type type: (type_identifier) @impl_type) body: (declaration_list (function_item) @fn_in_impl))",
        ] {
            if let Ok(q) = tree_sitter::Query::new(&lang, impl_q_src) {
                let type_idx  = q.capture_index_for_name("impl_type").unwrap();
                let fn_in_idx = q.capture_index_for_name("fn_in_impl").unwrap();
                let mut cursor = tree_sitter::QueryCursor::new();
                for m in cursor.matches(&q, tree.root_node(), source.as_bytes()) {
                    let tn  = m.captures.iter().find(|c| c.index == type_idx);
                    let fn_n = m.captures.iter().find(|c| c.index == fn_in_idx);
                    if let (Some(t), Some(f)) = (tn, fn_n) {
                        impl_type_map.insert(f.node.start_byte(), source[t.node.byte_range()].to_string());
                    }
                }
            }
        }

        // ── trait_impl map: struct name → [trait names] ──────────────────────
        // Populated from `impl Trait for Struct` blocks.
        let mut trait_impl_map: HashMap<String, Vec<String>> = HashMap::new();
        for impl_q_src in [
            "(impl_item trait: (type_identifier) @trait type: (type_identifier) @struct_type)",
            "(impl_item trait: (type_identifier) @trait type: (generic_type type: (type_identifier) @struct_type))",
            "(impl_item trait: (generic_type type: (type_identifier) @trait) type: (type_identifier) @struct_type)",
            "(impl_item trait: (generic_type type: (type_identifier) @trait) type: (generic_type type: (type_identifier) @struct_type))",
        ] {
            if let Ok(q) = tree_sitter::Query::new(&lang, impl_q_src) {
                let trait_idx  = q.capture_index_for_name("trait").unwrap();
                let struct_idx = q.capture_index_for_name("struct_type").unwrap();
                let mut cursor = tree_sitter::QueryCursor::new();
                for m in cursor.matches(&q, tree.root_node(), source.as_bytes()) {
                    let trait_cap  = m.captures.iter().find(|c| c.index == trait_idx);
                    let struct_cap = m.captures.iter().find(|c| c.index == struct_idx);
                    if let (Some(tr), Some(st)) = (trait_cap, struct_cap) {
                        let trait_name  = source[tr.node.byte_range()].to_string();
                        let struct_name = source[st.node.byte_range()].to_string();
                        trait_impl_map.entry(struct_name).or_default().push(trait_name);
                    }
                }
            }
        }

        // ── Pre-built call query (reused per function) ───────────────────────
        // Covers free calls `foo()`, method calls `obj.method()`,
        // and associated-function calls `Foo::bar()`.
        let call_q = tree_sitter::Query::new(
            &lang,
            "[(call_expression function: (identifier) @callee)
              (call_expression function: (field_expression field: (field_identifier) @callee))
              (call_expression function: (scoped_identifier name: (identifier) @callee))]",
        ).expect("valid Rust call query");
        let callee_idx = call_q.capture_index_for_name("callee").unwrap();

        // ── Pre-built field-type query (reused per struct/enum) ───────────────
        let field_q = tree_sitter::Query::new(
            &lang,
            "(field_declaration type: (_) @field_type)",
        ).expect("valid Rust field type query");
        let ft_idx = field_q.capture_index_for_name("field_type").unwrap();

        // ── Functions ────────────────────────────────────────────────────────
        let fn_q = tree_sitter::Query::new(
            &lang,
            "(function_item name: (identifier) @name parameters: (parameters) @params return_type: (_)? @ret) @fn",
        ).expect("valid Rust function query");
        let fn_idx   = fn_q.capture_index_for_name("fn").unwrap();
        let name_idx = fn_q.capture_index_for_name("name").unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&fn_q, tree.root_node(), source.as_bytes()) {
            let fn_node   = m.captures.iter().find(|c| c.index == fn_idx).map(|c| c.node);
            let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);
            if let (Some(fn_n), Some(nm_n)) = (fn_node, name_node) {
                let name      = &source[nm_n.byte_range()];
                let start_row = fn_n.start_position().row as u32 + 1;
                let end_row   = fn_n.end_position().row as u32 + 1;
                let src_text  = &source[fn_n.byte_range()];
                let impl_type = impl_type_map.get(&fn_n.start_byte()).cloned();
                let kind      = if impl_type.is_some() { "method" } else { "function" };

                let mut calls = Vec::new();
                let mut call_cur = tree_sitter::QueryCursor::new();
                for cm in call_cur.matches(&call_q, fn_n, source.as_bytes()) {
                    if let Some(cap) = cm.captures.iter().find(|c| c.index == callee_idx) {
                        calls.push(CallRef {
                            callee: source[cap.node.byte_range()].to_string(),
                            line:   cap.node.start_position().row as u32 + 1,
                        });
                    }
                }

                out.functions.push(FunctionNode {
                    repo: repo.into(),
                    version: version.into(),
                    file: file_path.clone(),
                    name: name.into(),
                    kind: kind.into(),
                    signature: first_line(src_text),
                    start_line: start_row,
                    end_line: end_row,
                    source: src_text.into(),
                    impl_type,
                    calls,
                });
            }
        }

        // ── Structs, enums, traits → ClassNode ───────────────────────────────
        for (type_q_src, kind) in [
            ("(struct_item name: (type_identifier) @name) @node", "struct"),
            ("(enum_item   name: (type_identifier) @name) @node", "enum"),
            ("(trait_item  name: (type_identifier) @name) @node", "trait"),
        ] {
            if let Ok(tq) = tree_sitter::Query::new(&lang, type_q_src) {
                let node_idx = tq.capture_index_for_name("node").unwrap();
                let name_idx = tq.capture_index_for_name("name").unwrap();
                let mut cursor = tree_sitter::QueryCursor::new();
                for m in cursor.matches(&tq, tree.root_node(), source.as_bytes()) {
                    let type_node = m.captures.iter().find(|c| c.index == node_idx).map(|c| c.node);
                    let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);
                    if let (Some(tn), Some(nm)) = (type_node, name_node) {
                        let name       = &source[nm.byte_range()];
                        let start_line = tn.start_position().row as u32 + 1;
                        let end_line   = tn.end_position().row as u32 + 1;
                        let src_text   = &source[tn.byte_range()];

                        // Merge explicit impl-for traits with #[derive(...)] traits.
                        let mut traits = trait_impl_map.get(name).cloned().unwrap_or_default();
                        traits.extend(rust_get_derives(source, tn));

                        // Field types from struct/enum bodies → USES edges.
                        let uses = rust_extract_field_uses(source, tn, name, &field_q, ft_idx);

                        out.classes.push(ClassNode {
                            repo: repo.into(),
                            version: version.into(),
                            file: file_path.clone(),
                            name: name.into(),
                            kind: kind.into(),
                            start_line,
                            end_line,
                            source: src_text.into(),
                            bases: vec![],
                            traits,
                            embeds: vec![],
                            uses,
                        });
                    }
                }
            }
        }

        out
    }
}

// ─────────────────────────────────────────────────────────────────────────────

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

        // Pre-build call query; reused per function to extract callee names.
        let call_q = tree_sitter::Query::new(
            &lang,
            "[(call function: (identifier) @callee)
              (call function: (attribute attribute: (identifier) @callee))]",
        ).expect("valid Python call query");
        let callee_idx = call_q.capture_index_for_name("callee").unwrap();

        // ── Functions and methods ─────────────────────────────────────────────
        let fn_q = tree_sitter::Query::new(
            &lang,
            "(function_definition name: (identifier) @name) @fn",
        ).expect("valid Python function query");
        let fn_idx   = fn_q.capture_index_for_name("fn").unwrap();
        let name_idx = fn_q.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&fn_q, tree.root_node(), source.as_bytes()) {
                let fn_node   = m.captures.iter().find(|c| c.index == fn_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);
                if let (Some(fn_n), Some(nm_n)) = (fn_node, name_node) {
                    let name       = &source[nm_n.byte_range()];
                    let start_line = fn_n.start_position().row as u32 + 1;
                    let end_line   = fn_n.end_position().row as u32 + 1;
                    let src_text   = &source[fn_n.byte_range()];
                    let kind = if python_has_class_ancestor(fn_n) { "method" } else { "function" };

                    // Extract outgoing calls within this function body.
                    let mut calls = Vec::new();
                    let mut call_cursor = tree_sitter::QueryCursor::new();
                    for cm in call_cursor.matches(&call_q, fn_n, source.as_bytes()) {
                        if let Some(cap) = cm.captures.iter().find(|c| c.index == callee_idx) {
                            let callee = source[cap.node.byte_range()].to_string();
                            let line   = cap.node.start_position().row as u32 + 1;
                            calls.push(CallRef { callee, line });
                        }
                    }

                    out.functions.push(FunctionNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file_path.clone(),
                        name: name.into(),
                        kind: kind.into(),
                        signature: first_line(src_text),
                        start_line,
                        end_line,
                        source: src_text.into(),
                        impl_type: None,
                        calls,
                    });
                }
            }
        }

        // ── Classes ──────────────────────────────────────────────────────────
        let cls_q = tree_sitter::Query::new(
            &lang,
            "(class_definition name: (identifier) @name) @class",
        ).expect("valid Python class query");
        let cls_idx  = cls_q.capture_index_for_name("class").unwrap();
        let cnm_idx  = cls_q.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&cls_q, tree.root_node(), source.as_bytes()) {
                let cls_node  = m.captures.iter().find(|c| c.index == cls_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == cnm_idx).map(|c| c.node);
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
                        kind: "class".into(),
                        start_line,
                        end_line,
                        source: src_text.into(),
                        bases: vec![],
                        traits: vec![],
                        embeds: vec![],
                        uses: vec![],
                    });
                }
            }
        }

        // ── Inheritance (base classes) ────────────────────────────────────────
        let inherit_q = tree_sitter::Query::new(
            &lang,
            "(class_definition name: (identifier) @class_name superclasses: (argument_list (identifier) @base_name))",
        ).expect("valid Python inheritance query");
        let iclass_idx = inherit_q.capture_index_for_name("class_name").unwrap();
        let ibase_idx  = inherit_q.capture_index_for_name("base_name").unwrap();
        {
            let mut bases_map: HashMap<String, Vec<String>> = HashMap::new();
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&inherit_q, tree.root_node(), source.as_bytes()) {
                let cn = m.captures.iter().find(|c| c.index == iclass_idx);
                let bn = m.captures.iter().find(|c| c.index == ibase_idx);
                if let (Some(cn), Some(bn)) = (cn, bn) {
                    bases_map
                        .entry(source[cn.node.byte_range()].to_string())
                        .or_default()
                        .push(source[bn.node.byte_range()].to_string());
                }
            }
            for cls in &mut out.classes {
                if let Some(bases) = bases_map.get(&cls.name) {
                    cls.bases = bases.clone();
                }
            }
        }

        // ── Imports ──────────────────────────────────────────────────────────
        let imp_q = tree_sitter::Query::new(
            &lang,
            "[(import_statement) (import_from_statement)] @import",
        ).expect("valid Python import query");
        let imp_idx = imp_q.capture_index_for_name("import").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&imp_q, tree.root_node(), source.as_bytes()) {
                if let Some(cap) = m.captures.iter().find(|c| c.index == imp_idx) {
                    let line   = cap.node.start_position().row as u32 + 1;
                    let target = python_import_target(source, cap.node);
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

// ─────────────────────────────────────────────────────────────────────────────

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn language_name(&self) -> &str { "typescript" }
    fn extensions(&self) -> &[&'static str] { &["ts", "tsx"] }

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_typescript::language_typescript()).unwrap();
        ParsedFile { path: path.to_string_lossy().into_owned(), language: "typescript".into(), ..Default::default() }
    }
}

pub struct JavaScriptParser;

impl LanguageParser for JavaScriptParser {
    fn language_name(&self) -> &str { "javascript" }
    fn extensions(&self) -> &[&'static str] { &["js", "jsx", "mjs"] }

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_javascript::language()).unwrap();
        ParsedFile { path: path.to_string_lossy().into_owned(), language: "javascript".into(), ..Default::default() }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

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

        // ── Pre-built call query ──────────────────────────────────────────────
        // Direct calls `foo()` and selector calls `obj.Method()` / `pkg.Func()`.
        let call_q = tree_sitter::Query::new(
            &lang,
            "[(call_expression function: (identifier) @callee)
              (call_expression function: (selector_expression field: (field_identifier) @callee))]",
        ).expect("valid Go call query");
        let callee_idx = call_q.capture_index_for_name("callee").unwrap();

        // ── Receiver-type map: method start_byte → receiver type name ─────────
        let mut receiver_map: HashMap<usize, String> = HashMap::new();
        let method_q = tree_sitter::Query::new(&lang, "(method_declaration) @m")
            .expect("valid Go method query");
        let m_idx = method_q.capture_index_for_name("m").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for mat in cursor.matches(&method_q, tree.root_node(), source.as_bytes()) {
                let mn = mat.captures.iter().find(|c| c.index == m_idx).map(|c| c.node);
                if let Some(mn) = mn {
                    if let Some(rt) = go_receiver_type_name(source, mn) {
                        receiver_map.insert(mn.start_byte(), rt.to_string());
                    }
                }
            }
        }

        // ── Functions and methods ─────────────────────────────────────────────
        let fn_q = tree_sitter::Query::new(
            &lang,
            "[(function_declaration name: (identifier) @name) @fn
              (method_declaration   name: (field_identifier) @name) @fn]",
        ).expect("valid Go function query");
        let fn_idx   = fn_q.capture_index_for_name("fn").unwrap();
        let name_idx = fn_q.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&fn_q, tree.root_node(), source.as_bytes()) {
                let fn_node   = m.captures.iter().find(|c| c.index == fn_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);
                if let (Some(fn_n), Some(nm_n)) = (fn_node, name_node) {
                    let name       = &source[nm_n.byte_range()];
                    let start_line = fn_n.start_position().row as u32 + 1;
                    let end_line   = fn_n.end_position().row as u32 + 1;
                    let src_text   = &source[fn_n.byte_range()];
                    let impl_type  = receiver_map.get(&fn_n.start_byte()).cloned();
                    let kind       = if impl_type.is_some() { "method" } else { "function" };

                    let mut calls = Vec::new();
                    let mut call_cur = tree_sitter::QueryCursor::new();
                    for cm in call_cur.matches(&call_q, fn_n, source.as_bytes()) {
                        if let Some(cap) = cm.captures.iter().find(|c| c.index == callee_idx) {
                            calls.push(CallRef {
                                callee: source[cap.node.byte_range()].to_string(),
                                line:   cap.node.start_position().row as u32 + 1,
                            });
                        }
                    }

                    out.functions.push(FunctionNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file_path.clone(),
                        name: name.into(),
                        kind: kind.into(),
                        signature: first_line(src_text),
                        start_line,
                        end_line,
                        source: src_text.into(),
                        impl_type,
                        calls,
                    });
                }
            }
        }

        // ── Structs and interfaces ────────────────────────────────────────────
        let type_q = tree_sitter::Query::new(
            &lang,
            "(type_spec name: (type_identifier) @name) @type_spec",
        ).expect("valid Go type query");
        let ts_idx  = type_q.capture_index_for_name("type_spec").unwrap();
        let tnm_idx = type_q.capture_index_for_name("name").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&type_q, tree.root_node(), source.as_bytes()) {
                let ts_node   = m.captures.iter().find(|c| c.index == ts_idx).map(|c| c.node);
                let name_node = m.captures.iter().find(|c| c.index == tnm_idx).map(|c| c.node);
                if let (Some(ts_n), Some(nm_n)) = (ts_node, name_node) {
                    // Determine kind from which child type is present; skip type aliases.
                    let kind = ts_n.children(&mut ts_n.walk()).find_map(|c| match c.kind() {
                        "struct_type"    => Some("struct"),
                        "interface_type" => Some("interface"),
                        _ => None,
                    });
                    let Some(kind) = kind else { continue };

                    let name       = &source[nm_n.byte_range()];
                    let start_line = ts_n.start_position().row as u32 + 1;
                    let end_line   = ts_n.end_position().row as u32 + 1;
                    let src_text   = &source[ts_n.byte_range()];

                    // Collect anonymously embedded types for struct nodes.
                    let embeds = if kind == "struct" {
                        go_struct_embedded_types(source, ts_n)
                    } else {
                        vec![]
                    };

                    out.classes.push(ClassNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file_path.clone(),
                        name: name.into(),
                        kind: kind.into(),
                        start_line,
                        end_line,
                        source: src_text.into(),
                        bases: vec![],
                        traits: vec![],
                        embeds,
                        uses: vec![],
                    });
                }
            }
        }

        // ── Imports ──────────────────────────────────────────────────────────
        let imp_q = tree_sitter::Query::new(
            &lang,
            "(import_spec path: (interpreted_string_literal) @path)",
        ).expect("valid Go import query");
        let path_idx = imp_q.capture_index_for_name("path").unwrap();
        {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&imp_q, tree.root_node(), source.as_bytes()) {
                if let Some(cap) = m.captures.iter().find(|c| c.index == path_idx) {
                    let line   = cap.node.start_position().row as u32 + 1;
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

// ─────────────────────────────────────────────────────────────────────────────

pub struct CParser;

impl LanguageParser for CParser {
    fn language_name(&self) -> &str { "c" }
    fn extensions(&self) -> &[&'static str] { &["c", "h"] }

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_c::language()).unwrap();
        ParsedFile { path: path.to_string_lossy().into_owned(), language: "c".into(), ..Default::default() }
    }
}

pub struct CppParser;

impl LanguageParser for CppParser {
    fn language_name(&self) -> &str { "cpp" }
    fn extensions(&self) -> &[&'static str] { &["cpp", "cc", "cxx", "hpp"] }

    fn parse(&self, _source: &str, path: &Path, _repo: &str, _version: &str) -> ParsedFile {
        let mut ts_parser = tree_sitter::Parser::new();
        ts_parser.set_language(&tree_sitter_cpp::language()).unwrap();
        ParsedFile { path: path.to_string_lossy().into_owned(), language: "cpp".into(), ..Default::default() }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// True if any ancestor of `node` is a `class_definition` (Python method detection).
fn python_has_class_ancestor(mut node: tree_sitter::Node) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == "class_definition" { return true; }
        node = parent;
    }
    false
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
                        if let Some(n) = child.child_by_field_name("name") {
                            return source[n.byte_range()].trim().to_string();
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

/// Walk a Go `method_declaration` receiver to extract the base type name.
/// Handles value (`Foo`), pointer (`*Foo`), and generic (`Foo[T]`, `*Foo[T]`) receivers.
fn go_receiver_type_name<'a>(source: &'a str, method_node: tree_sitter::Node<'a>) -> Option<&'a str> {
    let receiver = method_node.child_by_field_name("receiver")?;
    let mut walk = receiver.walk();
    for child in receiver.named_children(&mut walk) {
        if child.kind() == "parameter_declaration" {
            let type_node = child.child_by_field_name("type")?;
            return go_base_type_name(source, type_node);
        }
    }
    None
}

/// Peel `*` (pointer) and generic type arguments to reach the base `type_identifier`.
fn go_base_type_name<'a>(source: &'a str, node: tree_sitter::Node<'a>) -> Option<&'a str> {
    match node.kind() {
        "type_identifier" => Some(&source[node.byte_range()]),
        "pointer_type"    => go_base_type_name(source, node.named_child(0)?),
        "generic_type"    => go_base_type_name(source, node.child_by_field_name("type")?),
        _ => None,
    }
}

/// Collect the names of types anonymously embedded in a Go struct.
/// An embedded (anonymous) field has a `type` field but no `name` field.
fn go_struct_embedded_types(source: &str, type_spec_node: tree_sitter::Node) -> Vec<String> {
    let mut embeds = Vec::new();
    let mut spec_walk = type_spec_node.walk();
    for child in type_spec_node.named_children(&mut spec_walk) {
        if child.kind() != "struct_type" { continue; }
        let mut struct_walk = child.walk();
        for list_child in child.named_children(&mut struct_walk) {
            if list_child.kind() != "field_declaration_list" { continue; }
            let mut list_walk = list_child.walk();
            for field in list_child.named_children(&mut list_walk) {
                if field.kind() != "field_declaration" { continue; }
                // Embedded field: has a type but no name.
                if field.child_by_field_name("name").is_none() {
                    if let Some(type_node) = field.child_by_field_name("type") {
                        if let Some(name) = go_base_type_name(source, type_node) {
                            embeds.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    embeds
}

// ── Rust helpers ─────────────────────────────────────────────────────────────

/// Walk backwards from a struct/enum/trait node to collect trait names from
/// preceding `#[derive(...)]` attribute items.
fn rust_get_derives(source: &str, type_item: tree_sitter::Node) -> Vec<String> {
    let mut derives = Vec::new();
    let mut prev = type_item.prev_named_sibling();
    while let Some(node) = prev {
        if node.kind() != "attribute_item" { break; }
        derives.extend(rust_derives_from_attr(source, node));
        prev = node.prev_named_sibling();
    }
    derives
}

/// Extract trait names from a `#[derive(Trait1, Trait2)]` attribute_item node.
fn rust_derives_from_attr(source: &str, attr_item: tree_sitter::Node) -> Vec<String> {
    let mut result = Vec::new();
    let mut walk = attr_item.walk();
    for attr in attr_item.named_children(&mut walk) {
        if attr.kind() != "attribute" { continue; }
        // The first named child of `attribute` is the path identifier (e.g. "derive").
        let Some(path) = attr.named_child(0) else { continue };
        if source[path.byte_range()].trim() != "derive" { continue; }
        // Remaining named children include the token_tree with trait names.
        let mut a_walk = attr.walk();
        for sub in attr.named_children(&mut a_walk) {
            if sub.kind() == "token_tree" {
                let mut tt_walk = sub.walk();
                for ident in sub.named_children(&mut tt_walk) {
                    if ident.kind() == "identifier" {
                        result.push(source[ident.byte_range()].to_string());
                    }
                }
            }
        }
    }
    result
}

/// Collect user-defined type names from a Rust type expression recursively.
/// Only returns names starting with an uppercase letter (excludes primitives).
fn rust_collect_type_names(source: &str, node: tree_sitter::Node, out: &mut Vec<String>) {
    match node.kind() {
        "type_identifier" => {
            let name = &source[node.byte_range()];
            if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                out.push(name.to_string());
            }
        }
        // For `path::Type` scoped references, only the leaf name matters.
        "scoped_type_identifier" => {
            if let Some(name_child) = node.child_by_field_name("name") {
                rust_collect_type_names(source, name_child, out);
            }
        }
        _ => {
            let mut walk = node.walk();
            for child in node.named_children(&mut walk) {
                rust_collect_type_names(source, child, out);
            }
        }
    }
}

/// Extract unique user-defined type names referenced in field declarations within
/// a struct or enum node. Self-references (same name as the type itself) are excluded.
fn rust_extract_field_uses(
    source: &str,
    type_item: tree_sitter::Node,
    type_name: &str,
    field_q: &tree_sitter::Query,
    ft_idx: u32,
) -> Vec<String> {
    let mut uses = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(type_name.to_string()); // exclude self-references
    let mut cursor = tree_sitter::QueryCursor::new();
    for m in cursor.matches(field_q, type_item, source.as_bytes()) {
        if let Some(cap) = m.captures.iter().find(|c| c.index == ft_idx) {
            let mut types = Vec::new();
            rust_collect_type_names(source, cap.node, &mut types);
            for t in types {
                if seen.insert(t.clone()) {
                    uses.push(t);
                }
            }
        }
    }
    uses
}

fn first_line(text: &str) -> String {
    text.lines().find(|l| !l.trim().is_empty()).unwrap_or(text).trim().to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;
    use super::*;
    use crate::parser::LanguageParser;

    fn parse_rust(source: &str) -> crate::graph::model::ParsedFile {
        RustParser.parse(source, Path::new("src/lib.rs"), "myrepo", "v1.0")
    }

    fn parse_python(source: &str) -> crate::graph::model::ParsedFile {
        PythonParser.parse(source, Path::new("module.py"), "myrepo", "v1.0")
    }

    fn parse_go(source: &str) -> crate::graph::model::ParsedFile {
        GoParser.parse(source, Path::new("main.go"), "myrepo", "v1.0")
    }

    // ── Rust ──────────────────────────────────────────────────────────────────

    #[test] fn rust_single_function() {
        let pf = parse_rust(r#"fn hello() { println!("hi"); }"#);
        assert_eq!(pf.functions.len(), 1);
        let f = &pf.functions[0];
        assert_eq!(f.name, "hello");
        assert_eq!(f.kind, "function");
        assert_eq!(f.start_line, 1);
        assert_eq!(f.repo, "myrepo");
        assert_eq!(f.version, "v1.0");
    }

    #[test] fn rust_two_functions() {
        let pf = parse_rust("fn foo() {}\nfn bar() {}");
        assert_eq!(pf.functions.len(), 2);
        assert!(pf.functions.iter().all(|f| f.kind == "function"));
    }

    #[test] fn rust_function_line_numbers() {
        let pf = parse_rust("struct S;\n\nfn third_line() {}");
        assert_eq!(pf.functions[0].start_line, 3);
        assert_eq!(pf.functions[0].end_line, 3);
    }

    #[test] fn rust_function_with_return_type_signature() {
        let pf = parse_rust("fn add(a: i32, b: i32) -> i32 { a + b }");
        let sig = &pf.functions[0].signature;
        assert!(sig.contains("fn add") && sig.contains("->"), "sig: {sig}");
    }

    #[test] fn rust_empty_file() {
        let pf = parse_rust("");
        assert!(pf.functions.is_empty() && pf.classes.is_empty());
    }

    #[test] fn rust_parsed_file_metadata() {
        let pf = parse_rust("fn f() {}");
        assert_eq!(pf.language, "rust");
        assert_eq!(pf.path, "src/lib.rs");
    }

    #[test] fn rust_multiline_function_end_line() {
        let pf = parse_rust("fn multi() {\n    let x = 1;\n    let y = 2;\n}");
        assert_eq!(pf.functions[0].start_line, 1);
        assert_eq!(pf.functions[0].end_line, 4);
    }

    #[test] fn rust_source_text_captured() {
        let src = "fn greet() { \"hello\" }";
        let pf = parse_rust(src);
        assert_eq!(pf.functions[0].source, src);
    }

    #[test] fn rust_struct_extracted_as_class() {
        let pf = parse_rust("pub struct Foo { x: i32 }");
        assert_eq!(pf.classes.len(), 1);
        assert_eq!(pf.classes[0].name, "Foo");
        assert_eq!(pf.classes[0].kind, "struct");
    }

    #[test] fn rust_enum_extracted_as_class() {
        let pf = parse_rust("pub enum Color { Red, Green, Blue }");
        assert_eq!(pf.classes.len(), 1);
        assert_eq!(pf.classes[0].kind, "enum");
    }

    #[test] fn rust_trait_extracted_as_class() {
        let pf = parse_rust("pub trait Drawable { fn draw(&self); }");
        assert_eq!(pf.classes.len(), 1);
        assert_eq!(pf.classes[0].kind, "trait");
    }

    #[test] fn rust_impl_method_kind() {
        let pf = parse_rust("struct Foo;\nimpl Foo {\n    fn bar(&self) {}\n}");
        let bar = pf.functions.iter().find(|f| f.name == "bar").unwrap();
        assert_eq!(bar.kind, "method");
        assert_eq!(bar.impl_type.as_deref(), Some("Foo"));
    }

    #[test] fn rust_standalone_function_kind() {
        let pf = parse_rust("fn standalone() {}");
        assert_eq!(pf.functions[0].kind, "function");
        assert_eq!(pf.functions[0].impl_type, None);
    }

    #[test] fn rust_generic_impl_method() {
        let pf = parse_rust("struct Container<T>(T);\nimpl<T> Container<T> {\n    fn get(&self) {}\n}");
        let get = pf.functions.iter().find(|f| f.name == "get").unwrap();
        assert_eq!(get.kind, "method");
        assert_eq!(get.impl_type.as_deref(), Some("Container"));
    }

    #[test] fn rust_trait_impl_stored_on_struct() {
        let src = "struct Foo;\ntrait Bar {}\nimpl Bar for Foo {}";
        let pf = parse_rust(src);
        let foo = pf.classes.iter().find(|c| c.name == "Foo").unwrap();
        assert!(foo.traits.contains(&"Bar".to_string()), "traits: {:?}", foo.traits);
    }

    #[test] fn rust_generic_trait_impl() {
        let src = "struct Vec2;\ntrait From<T> {}\nimpl From<i32> for Vec2 {}";
        let pf = parse_rust(src);
        let v = pf.classes.iter().find(|c| c.name == "Vec2").unwrap();
        assert!(v.traits.contains(&"From".to_string()), "traits: {:?}", v.traits);
    }

    // ── Python ────────────────────────────────────────────────────────────────

    #[test] fn python_single_function() {
        let pf = parse_python("def hello():\n    pass");
        assert_eq!(pf.functions.len(), 1);
        assert_eq!(pf.functions[0].name, "hello");
        assert_eq!(pf.functions[0].kind, "function");
    }

    #[test] fn python_method_kind() {
        let src = "class Foo:\n    def bar(self):\n        pass";
        let pf = parse_python(src);
        let bar = pf.functions.iter().find(|f| f.name == "bar").unwrap();
        assert_eq!(bar.kind, "method");
    }

    #[test] fn python_standalone_function_kind() {
        let pf = parse_python("def standalone():\n    pass");
        assert_eq!(pf.functions[0].kind, "function");
    }

    #[test] fn python_class_kind() {
        let pf = parse_python("class Foo:\n    pass");
        assert_eq!(pf.classes[0].kind, "class");
    }

    #[test] fn python_calls_extracted() {
        let src = "def caller():\n    foo()\n    bar()";
        let pf = parse_python(src);
        let callees: Vec<&str> = pf.functions[0].calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"foo") && callees.contains(&"bar"), "callees: {callees:?}");
    }

    #[test] fn python_method_calls_extracted() {
        let src = "def f():\n    self.helper()\n    other()";
        let pf = parse_python(src);
        let callees: Vec<&str> = pf.functions[0].calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"helper") && callees.contains(&"other"), "callees: {callees:?}");
    }

    #[test] fn python_two_functions() {
        let pf = parse_python("def foo():\n    pass\n\ndef bar():\n    pass");
        assert_eq!(pf.functions.len(), 2);
    }

    #[test] fn python_class_definition() {
        let pf = parse_python("class Foo:\n    pass");
        assert_eq!(pf.classes.len(), 1);
        assert_eq!(pf.classes[0].name, "Foo");
    }

    #[test] fn python_single_inheritance() {
        let pf = parse_python("class Child(Parent):\n    pass");
        assert_eq!(pf.classes[0].bases, vec!["Parent"]);
    }

    #[test] fn python_multiple_inheritance() {
        let pf = parse_python("class Multi(Base1, Base2):\n    pass");
        let bases = &pf.classes[0].bases;
        assert!(bases.contains(&"Base1".to_string()) && bases.contains(&"Base2".to_string()));
    }

    #[test] fn python_no_inheritance() {
        let pf = parse_python("class Plain:\n    pass");
        assert!(pf.classes[0].bases.is_empty());
    }

    #[test] fn python_simple_import() {
        let pf = parse_python("import os");
        assert_eq!(pf.imports[0].target, "os");
    }

    #[test] fn python_from_import() {
        let pf = parse_python("from os import path");
        assert_eq!(pf.imports[0].target, "os");
    }

    #[test] fn python_empty_file() {
        let pf = parse_python("");
        assert!(pf.functions.is_empty() && pf.classes.is_empty() && pf.imports.is_empty());
    }

    // ── Go ────────────────────────────────────────────────────────────────────

    #[test] fn go_function_kind() {
        let pf = parse_go("package main\n\nfunc Hello() {}");
        assert_eq!(pf.functions[0].kind, "function");
    }

    #[test] fn go_method_kind() {
        let src = "package main\n\ntype Foo struct{}\n\nfunc (f Foo) Bar() {}";
        let pf = parse_go(src);
        let bar = pf.functions.iter().find(|f| f.name == "Bar").unwrap();
        assert_eq!(bar.kind, "method");
        assert_eq!(bar.impl_type.as_deref(), Some("Foo"));
    }

    #[test] fn go_pointer_receiver_kind() {
        let src = "package main\n\ntype Foo struct{}\n\nfunc (f *Foo) Bar() {}";
        let pf = parse_go(src);
        let bar = pf.functions.iter().find(|f| f.name == "Bar").unwrap();
        assert_eq!(bar.kind, "method");
        assert_eq!(bar.impl_type.as_deref(), Some("Foo"));
    }

    #[test] fn go_generic_receiver_type() {
        let src = "package main\n\ntype Stack[T any] struct{}\n\nfunc (s *Stack[T]) Push(v T) {}";
        let pf = parse_go(src);
        let push = pf.functions.iter().find(|f| f.name == "Push").unwrap();
        assert_eq!(push.impl_type.as_deref(), Some("Stack"));
    }

    #[test] fn go_struct_kind() {
        let src = "package main\n\ntype Foo struct {\n    X int\n}";
        let pf = parse_go(src);
        assert_eq!(pf.classes[0].kind, "struct");
    }

    #[test] fn go_interface_kind() {
        let src = "package main\n\ntype Reader interface {\n    Read() []byte\n}";
        let pf = parse_go(src);
        assert_eq!(pf.classes[0].kind, "interface");
    }

    #[test] fn go_struct_embed() {
        let src = "package main\n\ntype Base struct{}\n\ntype Child struct {\n    Base\n    X int\n}";
        let pf = parse_go(src);
        let child = pf.classes.iter().find(|c| c.name == "Child").unwrap();
        assert!(child.embeds.contains(&"Base".to_string()), "embeds: {:?}", child.embeds);
    }

    #[test] fn go_struct_pointer_embed() {
        let src = "package main\n\ntype Base struct{}\n\ntype Child struct {\n    *Base\n    X int\n}";
        let pf = parse_go(src);
        let child = pf.classes.iter().find(|c| c.name == "Child").unwrap();
        assert!(child.embeds.contains(&"Base".to_string()), "embeds: {:?}", child.embeds);
    }

    #[test] fn go_type_alias_excluded() {
        let src = "package main\n\ntype MyInt = int";
        let pf = parse_go(src);
        assert!(pf.classes.is_empty(), "type alias should not be a class");
    }

    #[test] fn go_single_import() {
        let src = "package main\nimport \"fmt\"\nfunc main() {}";
        let pf = parse_go(src);
        assert!(pf.imports.iter().any(|i| i.target == "fmt"));
    }

    #[test] fn go_empty_file() {
        let pf = parse_go("package main");
        assert!(pf.functions.is_empty() && pf.language == "go");
    }

    // ── Rust calls ────────────────────────────────────────────────────────────

    #[test] fn rust_free_call_extracted() {
        let src = "fn caller() { foo(); bar(); }";
        let pf = parse_rust(src);
        let callees: Vec<&str> = pf.functions.iter().find(|f| f.name == "caller").unwrap()
            .calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"foo") && callees.contains(&"bar"), "callees: {callees:?}");
    }

    #[test] fn rust_method_call_extracted() {
        let src = "fn f(obj: Foo) { obj.bar(); }";
        let pf = parse_rust(src);
        let callees: Vec<&str> = pf.functions[0].calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"bar"), "callees: {callees:?}");
    }

    #[test] fn rust_scoped_call_extracted() {
        let src = "fn f() { Vec::new(); Foo::create(); }";
        let pf = parse_rust(src);
        let callees: Vec<&str> = pf.functions[0].calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"new") && callees.contains(&"create"), "callees: {callees:?}");
    }

    // ── Rust derives ──────────────────────────────────────────────────────────

    #[test] fn rust_derive_augments_traits() {
        let src = "#[derive(Debug, Clone)]\nstruct Foo;";
        let pf = parse_rust(src);
        let foo = pf.classes.iter().find(|c| c.name == "Foo").unwrap();
        assert!(foo.traits.contains(&"Debug".to_string()), "traits: {:?}", foo.traits);
        assert!(foo.traits.contains(&"Clone".to_string()), "traits: {:?}", foo.traits);
    }

    #[test] fn rust_derive_and_impl_merged() {
        let src = "#[derive(Debug)]\nstruct Foo;\ntrait Bar {}\nimpl Bar for Foo {}";
        let pf = parse_rust(src);
        let foo = pf.classes.iter().find(|c| c.name == "Foo").unwrap();
        assert!(foo.traits.contains(&"Debug".to_string()) && foo.traits.contains(&"Bar".to_string()),
            "traits: {:?}", foo.traits);
    }

    // ── Rust field uses ───────────────────────────────────────────────────────

    #[test] fn rust_struct_field_uses_extracted() {
        let src = "struct Config;\nstruct Server { config: Config, count: usize }";
        let pf = parse_rust(src);
        let server = pf.classes.iter().find(|c| c.name == "Server").unwrap();
        assert!(server.uses.contains(&"Config".to_string()), "uses: {:?}", server.uses);
        // usize is a primitive, should not appear
        assert!(!server.uses.contains(&"usize".to_string()), "primitive leaked: {:?}", server.uses);
    }

    #[test] fn rust_generic_field_uses_extracted() {
        let src = "struct Peer;\nstruct Server { peers: Vec<Peer>, name: String }";
        let pf = parse_rust(src);
        let server = pf.classes.iter().find(|c| c.name == "Server").unwrap();
        assert!(server.uses.contains(&"Peer".to_string()), "uses: {:?}", server.uses);
    }

    #[test] fn rust_self_reference_excluded_from_uses() {
        let src = "struct Tree { left: Option<Box<Tree>>, value: i32 }";
        let pf = parse_rust(src);
        let tree = pf.classes.iter().find(|c| c.name == "Tree").unwrap();
        assert!(!tree.uses.contains(&"Tree".to_string()), "self-ref leaked: {:?}", tree.uses);
    }

    // ── Go calls ──────────────────────────────────────────────────────────────

    #[test] fn go_direct_call_extracted() {
        let src = "package main\n\nfunc caller() { foo() }";
        let pf = parse_go(src);
        let callees: Vec<&str> = pf.functions.iter().find(|f| f.name == "caller").unwrap()
            .calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"foo"), "callees: {callees:?}");
    }

    #[test] fn go_selector_call_extracted() {
        let src = "package main\n\nfunc f(obj *Foo) { obj.Bar() }";
        let pf = parse_go(src);
        let callees: Vec<&str> = pf.functions[0].calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(callees.contains(&"Bar"), "callees: {callees:?}");
    }

    // ── Stubs ─────────────────────────────────────────────────────────────────

    #[test] fn typescript_stub_returns_empty() {
        let pf = TypeScriptParser.parse("function foo() {}", Path::new("a.ts"), "r", "v1");
        assert!(pf.functions.is_empty() && pf.language == "typescript");
    }

    #[test] fn javascript_stub_returns_empty() {
        let pf = JavaScriptParser.parse("function foo() {}", Path::new("a.js"), "r", "v1");
        assert!(pf.functions.is_empty() && pf.language == "javascript");
    }

    #[test] fn c_stub_returns_empty() {
        let pf = CParser.parse("void foo() {}", Path::new("a.c"), "r", "v1");
        assert!(pf.functions.is_empty() && pf.language == "c");
    }

    #[test] fn cpp_stub_returns_empty() {
        let pf = CppParser.parse("void foo() {}", Path::new("a.cpp"), "r", "v1");
        assert!(pf.functions.is_empty() && pf.language == "cpp");
    }
}
