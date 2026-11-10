use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

use super::{is_test_marker, is_test_name, is_test_node, markers, statement_lines, LanguageParser};
use crate::graph::model::{CallRef, ClassNode, FunctionNode, ImportNode, ParsedFile};

pub struct Spec {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub language: fn() -> Language,
    pub tags: Option<&'static str>,
}

macro_rules! spec {
    ($name:literal, [$($ext:literal),*], $lang:expr, $tags:expr) => {
        Spec {
            name: $name,
            extensions: &[$($ext),*],
            language: || Language::from($lang),
            tags: $tags,
        }
    };
}

pub fn specs() -> Vec<Spec> {
    vec![
        spec!(
            "c",
            ["c", "h"],
            tree_sitter_c::LANGUAGE,
            Some(tree_sitter_c::TAGS_QUERY)
        ),
        spec!(
            "cpp",
            ["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl", "tpp"],
            tree_sitter_cpp::LANGUAGE,
            Some(tree_sitter_cpp::TAGS_QUERY)
        ),
        spec!(
            "java",
            ["java"],
            tree_sitter_java::LANGUAGE,
            Some(tree_sitter_java::TAGS_QUERY)
        ),
        spec!(
            "csharp",
            ["cs"],
            tree_sitter_c_sharp::LANGUAGE,
            Some(tree_sitter_c_sharp::TAGS_QUERY)
        ),
        spec!(
            "ruby",
            ["rb", "rake", "gemspec", "ru"],
            tree_sitter_ruby::LANGUAGE,
            Some(tree_sitter_ruby::TAGS_QUERY)
        ),
        spec!(
            "php",
            ["php", "phtml", "php5", "php7", "phpt"],
            tree_sitter_php::LANGUAGE_PHP,
            Some(tree_sitter_php::TAGS_QUERY)
        ),
        spec!(
            "kotlin",
            ["kt", "kts"],
            tree_sitter_kotlin_ng::LANGUAGE,
            None
        ),
        spec!(
            "scala",
            ["scala", "sc", "sbt"],
            tree_sitter_scala::LANGUAGE,
            None
        ),
        spec!(
            "swift",
            ["swift"],
            tree_sitter_swift::LANGUAGE,
            Some(tree_sitter_swift::TAGS_QUERY)
        ),
        spec!(
            "lua",
            ["lua"],
            tree_sitter_lua::LANGUAGE,
            Some(tree_sitter_lua::TAGS_QUERY)
        ),
        spec!(
            "bash",
            ["sh", "bash", "zsh", "ksh"],
            tree_sitter_bash::LANGUAGE,
            None
        ),
        spec!(
            "elixir",
            ["ex", "exs"],
            tree_sitter_elixir::LANGUAGE,
            Some(tree_sitter_elixir::TAGS_QUERY)
        ),
        spec!(
            "haskell",
            ["hs", "lhs"],
            tree_sitter_haskell::LANGUAGE,
            None
        ),
        spec!(
            "ocaml",
            ["ml"],
            tree_sitter_ocaml::LANGUAGE_OCAML,
            Some(tree_sitter_ocaml::TAGS_QUERY)
        ),
        spec!(
            "ocaml_interface",
            ["mli"],
            tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE,
            None
        ),
        spec!("julia", ["jl"], tree_sitter_julia::LANGUAGE, None),
        spec!(
            "elm",
            ["elm"],
            tree_sitter_elm::LANGUAGE,
            Some(tree_sitter_elm::TAGS_QUERY)
        ),
        spec!(
            "dart",
            ["dart"],
            tree_sitter_dart::LANGUAGE,
            Some(tree_sitter_dart::TAGS_QUERY)
        ),
        spec!(
            "r",
            ["r"],
            tree_sitter_r::LANGUAGE,
            Some(tree_sitter_r::TAGS_QUERY)
        ),
        spec!("zig", ["zig"], tree_sitter_zig::LANGUAGE, None),
        spec!("objc", ["mm"], tree_sitter_objc::LANGUAGE, None),
        spec!("erlang", ["erl", "hrl"], tree_sitter_erlang::LANGUAGE, None),
        spec!(
            "solidity",
            ["sol"],
            tree_sitter_solidity::LANGUAGE,
            Some(tree_sitter_solidity::TAGS_QUERY)
        ),
        spec!(
            "fortran",
            ["f", "for", "f77", "f90", "f95", "f03", "f08"],
            tree_sitter_fortran::LANGUAGE,
            None
        ),
        spec!(
            "groovy",
            ["groovy", "gradle", "gvy"],
            tree_sitter_groovy::LANGUAGE,
            None
        ),
        spec!("perl", ["pl", "pm", "t"], tree_sitter_perl::LANGUAGE, None),
        spec!("d", ["d", "di"], tree_sitter_d::LANGUAGE, None),
        spec!(
            "gleam",
            ["gleam"],
            tree_sitter_gleam::LANGUAGE,
            Some(tree_sitter_gleam::TAGS_QUERY)
        ),
        spec!(
            "fsharp",
            ["fs", "fsx"],
            tree_sitter_fsharp::LANGUAGE_FSHARP,
            Some(tree_sitter_fsharp::TAGS_QUERY)
        ),
        spec!(
            "fsharp_signature",
            ["fsi"],
            tree_sitter_fsharp::LANGUAGE_SIGNATURE,
            None
        ),
        spec!(
            "racket",
            ["rkt"],
            tree_sitter_racket::LANGUAGE,
            Some(tree_sitter_racket::TAGS_QUERY)
        ),
        spec!("proto", ["proto"], tree_sitter_proto::LANGUAGE, None),
        spec!(
            "verilog",
            ["v", "sv", "svh", "vh"],
            tree_sitter_verilog::LANGUAGE,
            None
        ),
        spec!(
            "pascal",
            ["pas", "pp", "dpr", "lpr"],
            tree_sitter_pascal::LANGUAGE,
            None
        ),
        spec!("nix", ["nix"], tree_sitter_nix::LANGUAGE, None),
        spec!(
            "powershell",
            ["ps1", "psm1"],
            tree_sitter_powershell::LANGUAGE,
            None
        ),
        spec!("cuda", ["cu", "cuh"], tree_sitter_cuda::LANGUAGE, None),
        spec!(
            "glsl",
            ["glsl", "vert", "frag", "geom", "comp", "tesc", "tese"],
            tree_sitter_glsl::LANGUAGE_GLSL,
            None
        ),
        spec!("scheme", ["scm", "ss"], tree_sitter_scheme::LANGUAGE, None),
        spec!(
            "ada",
            ["adb", "ads", "ada"],
            tree_sitter_ada::LANGUAGE,
            None
        ),
    ]
}

pub fn parsers() -> Vec<Arc<dyn LanguageParser>> {
    let mut parsers: Vec<Arc<dyn LanguageParser>> = specs()
        .into_iter()
        .map(|spec| Arc::new(GenericParser::new(spec)) as Arc<dyn LanguageParser>)
        .collect();
    parsers.push(Arc::new(ObjcOrMatlab::new()));
    parsers
}

pub struct GenericParser {
    spec: Spec,
    language: Language,
    tags: Option<Query>,
}

impl GenericParser {
    pub fn new(spec: Spec) -> Self {
        let language = (spec.language)();
        let tags = spec.tags.and_then(|src| match Query::new(&language, src) {
            Ok(query) => Some(query),
            Err(e) => {
                tracing::warn!(language = spec.name, error = %e, "tags query does not compile; using structure only");
                None
            }
        });
        Self {
            spec,
            language,
            tags,
        }
    }
}

impl LanguageParser for GenericParser {
    fn language_name(&self) -> &str {
        self.spec.name
    }

    fn extensions(&self) -> &[&'static str] {
        self.spec.extensions
    }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        extract(
            self.spec.name,
            &self.language,
            self.tags.as_ref(),
            source,
            path,
            repo,
            version,
        )
    }
}

pub struct ObjcOrMatlab {
    objc: GenericParser,
    matlab: GenericParser,
}

impl ObjcOrMatlab {
    fn new() -> Self {
        Self {
            objc: GenericParser::new(spec!("objc", ["m"], tree_sitter_objc::LANGUAGE, None)),
            matlab: GenericParser::new(spec!("matlab", ["m"], tree_sitter_matlab::LANGUAGE, None)),
        }
    }
}

fn looks_like_objc(source: &str) -> bool {
    [
        "@interface",
        "@implementation",
        "#import",
        "#include",
        "@protocol",
    ]
    .iter()
    .any(|m| source.contains(m))
}

impl LanguageParser for ObjcOrMatlab {
    fn language_name(&self) -> &str {
        "objc_or_matlab"
    }

    fn extensions(&self) -> &[&'static str] {
        &["m"]
    }

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile {
        if looks_like_objc(source) {
            self.objc.parse(source, path, repo, version)
        } else {
            self.matlab.parse(source, path, repo, version)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefKind {
    Class,
    Function,
}

const CLASS_WORDS: &[&str] = &[
    "class",
    "struct",
    "interface",
    "trait",
    "enum",
    "protocol",
    "module",
    "namespace",
    "object",
    "record",
    "contract",
    "library",
    "union",
    "impl",
    "extension",
    "mixin",
    "message",
    "service",
    "type_class",
    "instance",
    "actor",
];

const FUNCTION_WORDS: &[&str] = &[
    "function",
    "method",
    "procedure",
    "proc",
    "subroutine",
    "subprogram",
    "constructor",
    "destructor",
    "func",
    "fn",
    "fun",
    "def",
    "defn",
    "sub",
    "macro",
    "rpc",
    "operator",
    "initializer",
    "deinit",
    "getter",
    "setter",
];

const DEFINITION_ENDINGS: &[&str] = &[
    "declaration",
    "definition",
    "def",
    "decl",
    "item",
    "statement",
    "specifier",
    "implementation",
    "interface",
    "clause",
];

const NOT_DEFINITIONS: &[&str] = &[
    "call",
    "invocation",
    "parameter",
    "parameters",
    "argument",
    "arguments",
    "type",
    "reference",
    "pointer",
    "signature",
    "expression",
    "literal",
    "lambda",
    "arrow",
    "closure",
    "anonymous",
    "body",
    "list",
    "modifier",
    "modifiers",
    "name",
    "import",
    "comment",
    "declarator",
    "creation",
    "pattern",
    "field",
    "variant",
    "identifier",
    "specification",
    "constraint",
    "access",
    "prototype",
    "qualifier",
];

const EXPLICIT_DEFINITIONS: &[(&str, DefKind)] = &[
    ("subprogram_body", DefKind::Function),
    ("package_body", DefKind::Class),
    ("function_signature_item", DefKind::Function),
    ("fun_decl", DefKind::Function),
    ("module_definition", DefKind::Class),
    ("class_interface", DefKind::Class),
    ("class_implementation", DefKind::Class),
];

fn words(kind: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in kind.split(['_', '-', ' ']) {
        let mut current = String::new();
        for (i, c) in part.chars().enumerate() {
            if c.is_uppercase() && i > 0 && !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            current.push(c.to_ascii_lowercase());
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

fn classify(kind: &str) -> Option<DefKind> {
    if let Some((_, def)) = EXPLICIT_DEFINITIONS.iter().find(|(k, _)| *k == kind) {
        return Some(*def);
    }
    let parts = words(kind);
    if parts.is_empty() || parts.iter().any(|p| NOT_DEFINITIONS.contains(&p.as_str())) {
        return None;
    }
    let last = parts.last().map(String::as_str).unwrap_or("");
    let first = parts.first().map(String::as_str).unwrap_or("");
    let shaped = parts.len() == 1
        || DEFINITION_ENDINGS.contains(&last)
        || (parts.len() == 2 && DEFINITION_ENDINGS.contains(&first));
    if !shaped {
        return None;
    }
    let has = |list: &[&str]| parts.iter().any(|p| list.contains(&p.as_str()));
    if has(FUNCTION_WORDS) {
        Some(DefKind::Function)
    } else if has(CLASS_WORDS) {
        Some(DefKind::Class)
    } else {
        None
    }
}

fn tag_kind(capture: &str) -> Option<DefKind> {
    let kind = capture.strip_prefix("definition.")?;
    match kind {
        "function" | "method" | "macro" | "constructor" => Some(DefKind::Function),
        "class" | "interface" | "module" | "struct" | "enum" | "trait" | "type" | "union"
        | "object" | "protocol" | "namespace" | "contract" | "library" => Some(DefKind::Class),
        _ => None,
    }
}

fn is_name_kind(kind: &str) -> bool {
    kind.ends_with("identifier")
        || (kind.ends_with("_name") && !kind.starts_with("type"))
        || matches!(
            kind,
            "name"
                | "constant"
                | "word"
                | "symbol"
                | "atom"
                | "simple_identifier"
                | "variable"
                | "operator_name"
                | "destructor_name"
                | "property_identifier"
                | "method_name"
        )
}

const NAME_SKIP_WORDS: &[&str] = &[
    "parameter",
    "parameters",
    "argument",
    "arguments",
    "comment",
    "string",
    "decorator",
    "annotation",
    "attribute",
    "modifier",
    "modifiers",
    "superclass",
    "base",
    "extends",
    "implements",
    "return",
    "result",
    "call",
];

const NAME_SKIP_LAST: &[&str] = &["body", "block", "list", "statements", "group"];

fn skip_for_name(kind: &str) -> bool {
    let parts = words(kind);
    parts.iter().any(|p| NAME_SKIP_WORDS.contains(&p.as_str()))
        || parts
            .last()
            .is_some_and(|p| NAME_SKIP_LAST.contains(&p.as_str()))
}

fn first_name(source: &str, node: Node, depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }
    if is_name_kind(node.kind()) {
        let t = text(source, node).trim();
        return (!t.is_empty() && t.len() <= 200 && !t.contains('\n')).then(|| t.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    children.into_iter().find_map(|child| {
        let kind = child.kind();
        if kind.contains("argument") || kind.contains("parameter") {
            return None;
        }
        first_name(source, child, depth + 1)
    })
}

fn is_header_kind(kind: &str) -> bool {
    [
        "signature",
        "declarator",
        "header",
        "head",
        "prototype",
        "spec",
    ]
    .iter()
    .any(|w| words(kind).iter().any(|p| p == w))
}

fn text<'a>(source: &'a str, node: Node) -> &'a str {
    &source[node.byte_range()]
}

fn last_name(source: &str, node: Node) -> Option<String> {
    if node.named_child_count() == 0 {
        let t = text(source, node).trim();
        return (!t.is_empty() && t.len() <= 200).then(|| t.to_string());
    }
    let mut found = None;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let kind = child.kind();
        if kind.contains("argument")
            || kind.contains("parameter")
            || kind.contains("type_arguments")
        {
            continue;
        }
        if is_name_kind(kind) || child.named_child_count() > 0 {
            if let Some(name) = last_name(source, child) {
                found = Some(name);
            }
        }
    }
    found
}

fn declarator_name(source: &str, node: Node) -> Option<String> {
    if is_name_kind(node.kind()) {
        return Some(text(source, node).to_string());
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return declarator_name(source, inner);
    }
    if let Some(name) = node.child_by_field_name("name") {
        return last_name(source, name);
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    children
        .into_iter()
        .filter(|c| !c.kind().contains("parameter"))
        .find_map(|c| declarator_name(source, c))
}

fn find_name(source: &str, node: Node) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        if let Some(found) = last_name(source, name) {
            return Some(found);
        }
    }
    if let Some(declarator) = node.child_by_field_name("declarator") {
        if let Some(found) = declarator_name(source, declarator) {
            return Some(found);
        }
    }
    {
        let mut cursor = node.walk();
        let headers: Vec<Node> = node
            .named_children(&mut cursor)
            .take(4)
            .filter(|c| is_header_kind(c.kind()))
            .collect();
        for header in headers {
            if let Some(found) = first_name(source, header, 0) {
                return Some(found);
            }
        }
    }
    let mut frontier = vec![node];
    for _ in 0..3 {
        let mut next = Vec::new();
        for current in frontier {
            let mut cursor = current.walk();
            for (i, child) in current.named_children(&mut cursor).enumerate() {
                if i > 6 {
                    break;
                }
                let kind = child.kind();
                if skip_for_name(kind) {
                    continue;
                }
                if is_name_kind(kind) {
                    let t = text(source, child).trim();
                    if !t.is_empty() && t.len() <= 200 && !t.contains('\n') {
                        return Some(t.to_string());
                    }
                }
                next.push(child);
            }
        }
        frontier = next;
    }
    None
}

const BASE_WORDS: &[&str] = &[
    "superclass",
    "superclasses",
    "super",
    "base",
    "extends",
    "implements",
    "inherit",
    "inheritance",
    "heritage",
    "delegation",
    "interfaces",
    "parent",
    "conformance",
    "derive",
];

fn is_base_container(kind: &str, field: Option<&str>) -> bool {
    let mut parts = words(kind);
    if let Some(field) = field {
        parts.extend(words(field));
    }
    parts.iter().any(|p| BASE_WORDS.contains(&p.as_str()))
}

fn collect_type_names(source: &str, node: Node, out: &mut Vec<String>) {
    let kind = node.kind();
    if kind.contains("argument")
        && !kind.contains("superclass")
        && node
            .parent()
            .is_some_and(|p| !is_base_container(p.kind(), None))
    {
        return;
    }
    if node.named_child_count() == 0 {
        if is_name_kind(kind) {
            let name = text(source, node).trim().to_string();
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
        return;
    }
    if kind.contains("scoped")
        || kind.contains("qualified")
        || kind.contains("member")
        || kind.contains("generic")
    {
        if let Some(name) = last_name(source, node) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_type_names(source, child, out);
    }
}

fn find_bases(source: &str, node: Node, own: &str) -> Vec<String> {
    let mut bases = Vec::new();
    let mut cursor = node.walk();
    let mut pending: Vec<(Node, Option<&'static str>)> = Vec::new();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() {
                pending.push((child, cursor.field_name()));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    for (child, field) in pending {
        let kind = child.kind();
        if kind.contains("body") || kind.contains("block") || kind.contains("comment") {
            continue;
        }
        if is_base_container(kind, field) {
            collect_type_names(source, child, &mut bases);
        } else if !kind.contains("body") {
            let mut inner = child.walk();
            for grandchild in child.named_children(&mut inner) {
                if is_base_container(grandchild.kind(), None) {
                    collect_type_names(source, grandchild, &mut bases);
                }
            }
        }
    }
    bases.retain(|b| b != own && b.chars().any(char::is_alphabetic) && b.len() <= 120);
    bases
}

fn is_call_kind(kind: &str) -> bool {
    let parts = words(kind);
    if parts
        .iter()
        .any(|p| matches!(p.as_str(), "signature" | "type" | "declaration"))
    {
        return false;
    }
    parts
        .iter()
        .any(|p| matches!(p.as_str(), "call" | "invocation" | "invoke" | "send"))
        || kind == "message_expression"
        || kind == "command"
}

fn callee_name(source: &str, node: Node) -> Option<String> {
    for field in [
        "function",
        "method",
        "name",
        "callee",
        "selector",
        "macro",
        "command_name",
    ] {
        if let Some(target) = node.child_by_field_name(field) {
            if let Some(name) = last_name(source, target) {
                return Some(name);
            }
        }
    }
    let first = node.named_child(0)?;
    if first.kind().contains("argument") {
        return None;
    }
    last_name(source, first)
}

fn clean_comment(raw: &str) -> String {
    raw.lines()
        .map(|l| {
            l.trim()
                .trim_start_matches("/**")
                .trim_start_matches("/*")
                .trim_end_matches("*/")
                .trim_start_matches("///")
                .trim_start_matches("//!")
                .trim_start_matches("//")
                .trim_start_matches("--")
                .trim_start_matches(";;")
                .trim_start_matches(';')
                .trim_start_matches('#')
                .trim_start_matches('%')
                .trim_start_matches('*')
                .trim()
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn leading_comment(source: &str, node: Node) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut expected_end = node.start_position().row;
    let mut previous = node.prev_sibling();
    while let Some(sibling) = previous {
        if sibling.kind().contains("comment") && sibling.end_position().row + 1 >= expected_end {
            lines.push(text(source, sibling).to_string());
            expected_end = sibling.start_position().row;
            previous = sibling.prev_sibling();
        } else if super::markers(source, node).is_empty() || !sibling.is_named() {
            break;
        } else {
            previous = sibling.prev_sibling();
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    let doc = clean_comment(&lines.join("\n"));
    (!doc.is_empty()).then_some(doc)
}

fn is_import_kind(kind: &str) -> bool {
    let parts = words(kind);
    parts.iter().any(|p| {
        matches!(
            p.as_str(),
            "import" | "include" | "using" | "require" | "imports" | "open" | "uses"
        )
    }) && !parts
        .iter()
        .any(|p| matches!(p.as_str(), "call" | "expression" | "list"))
        || kind == "use_declaration"
        || kind == "preproc_include"
}

fn import_target(source: &str, node: Node) -> Option<String> {
    let mut cursor = node.walk();
    let best = node
        .named_children(&mut cursor)
        .filter(|c| !c.kind().contains("comment"))
        .last()?;
    let raw = text(source, best)
        .trim()
        .trim_matches(['"', '\'', '<', '>', ';', '`'])
        .trim();
    (!raw.is_empty() && raw.len() <= 200 && !raw.contains('\n')).then(|| raw.to_string())
}

struct Definition<'t> {
    node: Node<'t>,
    kind: DefKind,
    name: String,
}

fn first_line(body: &str) -> String {
    body.lines().next().unwrap_or("").trim_end().to_string()
}

pub fn extract(
    language_name: &str,
    language: &Language,
    tags: Option<&Query>,
    source: &str,
    path: &Path,
    repo: &str,
    version: &str,
) -> ParsedFile {
    let file = path.to_string_lossy().replace('\\', "/");
    let mut out = ParsedFile {
        path: file.clone(),
        language: language_name.into(),
        ..Default::default()
    };
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else {
        return out;
    };
    let root = tree.root_node();

    let mut definitions: Vec<Definition> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut tag_calls: Vec<(usize, String, u32)> = Vec::new();

    if let Some(query) = tags {
        let names = query.capture_names();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, root, source.as_bytes());
        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| names[c.index as usize] == "name")
                .map(|c| c.node);
            for capture in m.captures {
                let capture_name = names[capture.index as usize];
                if let Some(kind) = tag_kind(capture_name) {
                    let Some(name_node) = name_node else { continue };
                    if seen.insert(capture.node.id()) {
                        definitions.push(Definition {
                            node: capture.node,
                            kind,
                            name: text(source, name_node).trim().to_string(),
                        });
                    }
                } else if capture_name.starts_with("reference.") && !capture_name.ends_with("class")
                {
                    if let Some(name_node) = name_node {
                        tag_calls.push((
                            capture.node.start_byte(),
                            text(source, name_node).trim().to_string(),
                            capture.node.start_position().row as u32 + 1,
                        ));
                    }
                }
            }
        }
    }

    let mut structural_calls: Vec<(usize, String, u32)> = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if node.is_named() {
            if let Some(def) = classify(kind) {
                if !seen.contains(&node.id())
                    && !definitions
                        .iter()
                        .any(|d| d.node.byte_range() == node.byte_range())
                {
                    if let Some(name) = find_name(source, node) {
                        seen.insert(node.id());
                        definitions.push(Definition {
                            node,
                            kind: def,
                            name,
                        });
                    }
                }
            } else if is_call_kind(kind) {
                if let Some(name) = callee_name(source, node) {
                    structural_calls.push((
                        node.start_byte(),
                        name,
                        node.start_position().row as u32 + 1,
                    ));
                }
            } else if is_import_kind(kind)
                && node
                    .parent()
                    .is_some_and(|p| p.parent().is_none() || !is_import_kind(p.kind()))
            {
                if let Some(target) = import_target(source, node) {
                    out.imports.push(ImportNode {
                        repo: repo.into(),
                        version: version.into(),
                        file: file.clone(),
                        target,
                        line: node.start_position().row as u32 + 1,
                    });
                }
            }
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }

    definitions.retain(|d| {
        !d.name.is_empty()
            && d.name.len() <= 200
            && !d.name.contains('\n')
            && d.name.chars().any(|c| c.is_alphanumeric() || c == '_')
    });
    definitions.sort_by_key(|d| (d.node.start_byte(), std::cmp::Reverse(d.node.end_byte())));
    let mut unique: Vec<Definition> = Vec::new();
    for def in definitions {
        let duplicate = unique.iter().any(|u| {
            u.kind == def.kind
                && (u.node.byte_range() == def.node.byte_range()
                    || (u.name == def.name
                        && u.node.start_byte() <= def.node.start_byte()
                        && u.node.end_byte() >= def.node.end_byte()))
        });
        if duplicate {
            continue;
        }
        unique.push(def);
    }
    let definitions = unique;

    let owner_of = |node: Node| -> Option<String> {
        definitions
            .iter()
            .filter(|d| {
                d.kind == DefKind::Class
                    && d.node.id() != node.id()
                    && d.node.start_byte() <= node.start_byte()
                    && d.node.end_byte() >= node.end_byte()
            })
            .max_by_key(|d| d.node.start_byte())
            .map(|d| d.name.clone())
    };

    let function_ranges: Vec<(usize, usize, usize)> = definitions
        .iter()
        .enumerate()
        .filter(|(_, d)| d.kind == DefKind::Function)
        .map(|(i, d)| (i, d.node.start_byte(), d.node.end_byte()))
        .collect();
    let mut calls_by_def: HashMap<usize, Vec<CallRef>> = HashMap::new();
    for (start, name, line) in tag_calls.into_iter().chain(structural_calls) {
        let owner = function_ranges
            .iter()
            .filter(|(_, s, e)| *s <= start && start < *e)
            .max_by_key(|(_, s, _)| *s)
            .map(|(i, _, _)| *i);
        if let Some(owner) = owner {
            let list = calls_by_def.entry(owner).or_default();
            if !list.iter().any(|c| c.callee == name && c.line == line) {
                list.push(CallRef { callee: name, line });
            }
        }
    }

    let mut test_classes: HashSet<String> = HashSet::new();
    for def in definitions.iter().filter(|d| d.kind == DefKind::Class) {
        let body = text(source, def.node);
        let marks = markers(source, def.node);
        let bases = find_bases(source, def.node, &def.name);
        let is_test = is_test_node(source, def.node, &def.name)
            || bases
                .iter()
                .any(|b| is_test_name(b) || b.ends_with("TestCase"));
        if is_test {
            test_classes.insert(def.name.clone());
        }
        out.classes.push(ClassNode {
            repo: repo.into(),
            version: version.into(),
            file: file.clone(),
            name: def.name.clone(),
            kind: def.node.kind().into(),
            start_line: def.node.start_position().row as u32 + 1,
            end_line: def.node.end_position().row as u32 + 1,
            source: body.into(),
            bases,
            traits: Vec::new(),
            embeds: Vec::new(),
            uses: Vec::new(),
            docstring: leading_comment(source, def.node),
            decorators: marks,
            is_test,
        });
    }

    for (i, def) in definitions.iter().enumerate() {
        if def.kind != DefKind::Function {
            continue;
        }
        let body = text(source, def.node);
        let impl_type = owner_of(def.node);
        let is_test = is_test_node(source, def.node, &def.name)
            || impl_type.as_ref().is_some_and(|c| test_classes.contains(c))
            || markers(source, def.node).iter().any(|m| is_test_marker(m));
        out.functions.push(FunctionNode {
            repo: repo.into(),
            version: version.into(),
            file: file.clone(),
            name: def.name.clone(),
            kind: if impl_type.is_some() {
                "method"
            } else {
                "function"
            }
            .into(),
            signature: first_line(body),
            start_line: def.node.start_position().row as u32 + 1,
            end_line: def.node.end_position().row as u32 + 1,
            source: body.into(),
            impl_type,
            docstring: leading_comment(source, def.node),
            calls: calls_by_def.remove(&i).unwrap_or_default(),
            is_test,
            statement_lines: statement_lines(def.node),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached() -> &'static Vec<Arc<dyn LanguageParser>> {
        static PARSERS: std::sync::OnceLock<Vec<Arc<dyn LanguageParser>>> =
            std::sync::OnceLock::new();
        PARSERS.get_or_init(parsers)
    }

    #[test]
    fn query_compilation_is_fast() {
        for spec in specs() {
            let started = std::time::Instant::now();
            let _ = GenericParser::new(spec);
            assert!(started.elapsed().as_secs_f64() < 5.0);
        }
    }

    fn parse(ext: &str, source: &str) -> ParsedFile {
        let parsers = cached();
        let parser = parsers
            .iter()
            .find(|p| p.extensions().contains(&ext))
            .unwrap_or_else(|| panic!("no parser for {ext}"));
        parser.parse(source, Path::new(&format!("src/sample.{ext}")), "r", "v")
    }

    fn names(parsed: &ParsedFile) -> (Vec<String>, Vec<String>) {
        (
            parsed.classes.iter().map(|c| c.name.clone()).collect(),
            parsed
                .functions
                .iter()
                .map(|f| match &f.impl_type {
                    Some(owner) => format!("{owner}.{}", f.name),
                    None => f.name.clone(),
                })
                .collect(),
        )
    }

    #[test]
    fn every_grammar_loads_into_the_runtime() {
        for spec in specs() {
            let mut parser = Parser::new();
            assert!(
                parser.set_language(&(spec.language)()).is_ok(),
                "{} does not load",
                spec.name
            );
        }
        let _ = ObjcOrMatlab::new();
    }

    #[test]
    fn java_classes_methods_bases_calls_and_tests() {
        let parsed = parse(
            "java",
            "package a;\nimport java.util.List;\n/** Stores volumes. */\nclass Store extends Base implements Api {\n  void save(int x) {\n    helper(x);\n    other.run();\n  }\n}\nclass StoreTest {\n  @Test\n  void savesThings() { new Store().save(1); }\n}\n",
        );
        let (classes, functions) = names(&parsed);
        assert_eq!(classes, vec!["Store", "StoreTest"]);
        assert!(
            functions.contains(&"Store.save".to_string()),
            "{functions:?}"
        );
        let store = &parsed.classes[0];
        assert!(
            store.bases.contains(&"Base".to_string()),
            "{:?}",
            store.bases
        );
        assert!(
            store.bases.contains(&"Api".to_string()),
            "{:?}",
            store.bases
        );
        assert_eq!(store.docstring.as_deref(), Some("Stores volumes."));
        let save = parsed.functions.iter().find(|f| f.name == "save").unwrap();
        let callees: Vec<&str> = save.calls.iter().map(|c| c.callee.as_str()).collect();
        assert!(
            callees.contains(&"helper") && callees.contains(&"run"),
            "{callees:?}"
        );
        assert!(!save.is_test);
        assert_eq!(save.statement_lines, vec![6, 7]);
        let test = parsed
            .functions
            .iter()
            .find(|f| f.name == "savesThings")
            .unwrap();
        assert!(test.is_test);
        assert!(parsed.classes[1].is_test);
        assert!(
            parsed
                .imports
                .iter()
                .any(|i| i.target.contains("java.util.List")),
            "{:?}",
            parsed.imports
        );
    }

    #[test]
    fn c_and_cpp_functions_are_found() {
        let c = parse(
            "c",
            "#include <stdio.h>\nstatic int add(int a, int b) {\n  return helper(a) + b;\n}\n",
        );
        assert_eq!(names(&c).1, vec!["add"]);
        assert_eq!(c.functions[0].calls[0].callee, "helper");
        let cpp = parse("cpp", "namespace store {\nclass Volume : public Base {\n public:\n  void attach() { connect(); }\n};\n}\n");
        let (classes, functions) = names(&cpp);
        assert!(classes.contains(&"Volume".to_string()), "{classes:?}");
        assert!(
            functions.contains(&"Volume.attach".to_string()),
            "{functions:?}"
        );
        let volume = cpp.classes.iter().find(|c| c.name == "Volume").unwrap();
        assert_eq!(volume.bases, vec!["Base"]);
    }

    #[test]
    fn ruby_csharp_kotlin_and_go_like_languages() {
        let ruby = parse(
            "rb",
            "class Volume < Base\n  def attach\n    connect\n  end\nend\n",
        );
        assert_eq!(names(&ruby).1, vec!["Volume.attach"]);
        assert_eq!(ruby.classes[0].bases, vec!["Base"]);
        let cs = parse("cs", "namespace A {\n  public class Store : Base {\n    [Fact]\n    public void Saves() { Helper(); }\n  }\n}\n");
        let saves = cs.functions.iter().find(|f| f.name == "Saves").unwrap();
        assert!(saves.is_test);
        assert_eq!(saves.impl_type.as_deref(), Some("Store"));
        let kotlin = parse(
            "kt",
            "class Store : Base() {\n    fun save(x: Int) {\n        helper(x)\n    }\n}\n",
        );
        let (classes, functions) = names(&kotlin);
        assert_eq!(classes, vec!["Store"]);
        assert_eq!(functions, vec!["Store.save"]);
        assert_eq!(kotlin.functions[0].calls[0].callee, "helper");
    }

    #[test]
    fn many_languages_yield_definitions() {
        let samples: &[(&str, &str, &str)] = &[
            ("php", "<?php\nclass Store extends Base {\n  function save() { helper(); }\n}\n", "save"),
            ("scala", "class Store extends Base {\n  def save(): Unit = helper()\n}\n", "save"),
            ("swift", "class Store: Base {\n  func save() { helper() }\n}\n", "save"),
            ("lua", "local function save()\n  helper()\nend\n", "save"),
            ("sh", "save() {\n  helper\n}\n", "save"),
            ("ex", "defmodule Store do\n  def save(x), do: helper(x)\nend\n", "save"),
            ("hs", "save :: Int -> Int\nsave x = helper x\n", "save"),
            ("ml", "let save x = helper x\n", "save"),
            ("jl", "function save(x)\n  helper(x)\nend\n", "save"),
            ("dart", "class Store extends Base {\n  void save() { helper(); }\n}\n", "save"),
            ("r", "save <- function(x) {\n  helper(x)\n}\n", "save"),
            ("zig", "fn save() void {\n    helper();\n}\n", "save"),
            ("erl", "-module(store).\nsave(X) -> helper(X).\n", "save"),
            ("sol", "contract Store is Base {\n  function save() public { helper(); }\n}\n", "save"),
            ("f90", "subroutine save(x)\n  call helper(x)\nend subroutine save\n", "save"),
            ("groovy", "class Store {\n  def save() {\n    helper()\n  }\n}\n", "save"),
            ("pl", "package Store;\nsub save {\n  helper();\n}\n1;\n", "save"),
            ("d", "class Store : Base {\n  void save() { helper(); }\n}\n", "save"),
            ("gleam", "pub fn save(x) {\n  helper(x)\n}\n", "save"),
            ("fs", "let save x = helper x\n", "save"),
            ("rkt", "#lang racket\n(define (save x) (helper x))\n", "save"),
            ("proto", "syntax = \"proto3\";\nservice Store {\n  rpc Save (Req) returns (Resp);\n}\n", "Save"),
            ("pas", "program a;\nprocedure Save;\nbegin\n  Helper;\nend;\nbegin\nend.\n", "Save"),
            ("ps1", "function Save-Volume {\n  Get-Helper\n}\n", "Save-Volume"),
            ("cu", "__global__ void save(float *x) {\n  helper(x);\n}\n", "save"),
            ("glsl", "void save() {\n  helper();\n}\n", "save"),
            ("adb", "procedure Save is\nbegin\n  Helper;\nend Save;\n", "Save"),
            ("v", "module store(input a);\n  function save;\n    input x;\n    save = x;\n  endfunction\nendmodule\n", "save"),
            ("elm", "module Store exposing (..)\nsave x = helper x\n", "save"),
            ("mm", "@implementation Store\n- (void)save {\n  [self helper];\n}\n@end\n", "save"),
        ];
        let mut missing = Vec::new();
        for (ext, source, expected) in samples {
            let parsed = parse(ext, source);
            let (_, functions) = names(&parsed);
            if !functions
                .iter()
                .any(|f| f == expected || f.ends_with(&format!(".{expected}")))
            {
                missing.push(format!("{ext}: {functions:?}"));
            }
        }
        assert!(missing.is_empty(), "no definition found for {missing:#?}");
    }

    #[test]
    fn m_files_pick_objc_or_matlab_by_content() {
        let parser = ObjcOrMatlab::new();
        let objc = parser.parse(
            "#import <Foundation/Foundation.h>\n@implementation A\n- (void)f { }\n@end\n",
            Path::new("a.m"),
            "r",
            "v",
        );
        assert_eq!(objc.language, "objc");
        let matlab = parser.parse(
            "function y = f(x)\n  y = g(x);\nend\n",
            Path::new("a.m"),
            "r",
            "v",
        );
        assert_eq!(matlab.language, "matlab");
        assert_eq!(names(&matlab).1, vec!["f"]);
    }
}
