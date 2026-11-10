use std::collections::{HashMap, HashSet};

use crate::domain::UnitId;
use crate::index::corpus::{Edge, Relation, Unit, UnitKind};
use crate::index::tokenizer;

pub const MAX_TARGETS_PER_MENTION: usize = 3;
const MIN_MENTION_CHARS: usize = 3;

pub struct Names<'a> {
    by_name: &'a HashMap<String, Vec<UnitId>>,
    modules_by_file: HashMap<&'a str, UnitId>,
    modules_by_stem: HashMap<String, Vec<UnitId>>,
}

fn file_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name).to_lowercase()
}

impl<'a> Names<'a> {
    pub fn new(by_name: &'a HashMap<String, Vec<UnitId>>, units: &'a [Unit]) -> Self {
        let modules: Vec<&Unit> = units
            .iter()
            .filter(|u| u.kind() == UnitKind::Module)
            .collect();
        let modules_by_file = modules.iter().map(|u| (u.file.as_str(), u.id)).collect();
        let mut modules_by_stem: HashMap<String, Vec<UnitId>> = HashMap::new();
        for module in modules.iter().filter(|u| is_code_target(u)) {
            modules_by_stem
                .entry(file_stem(&module.file))
                .or_default()
                .push(module.id);
        }
        Self {
            by_name,
            modules_by_file,
            modules_by_stem,
        }
    }

    pub fn symbols(&self, name: &str) -> &'a [UnitId] {
        lookup(self.by_name, name)
    }

    fn modules_named(&self, word: &str) -> &[UnitId] {
        self.modules_by_stem
            .get(&word.to_lowercase())
            .map_or(&[], Vec::as_slice)
    }

    fn module_of(&self, path: &str) -> Option<UnitId> {
        let path = path.trim_start_matches("./");
        self.modules_by_file.get(path).copied().or_else(|| {
            self.modules_by_file
                .iter()
                .find(|(file, _)| path.contains('/') && file.ends_with(&format!("/{path}")))
                .map(|(_, id)| *id)
        })
    }
}

pub fn lookup<'a>(names: &'a HashMap<String, Vec<UnitId>>, name: &str) -> &'a [UnitId] {
    let parts = tokenizer::name_parts(name);
    for start in 0..parts.len() {
        if let Some(found) = names.get(&parts[start..].join(".")) {
            return found;
        }
    }
    &[]
}

fn backticked(text: &str) -> Vec<&str> {
    text.split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, inner)| inner.trim())
        .filter(|inner| !inner.is_empty() && !inner.contains('\n'))
        .collect()
}

fn looks_like_path(word: &str) -> bool {
    let last = word.rsplit('/').next().unwrap_or(word);
    word.contains('/') && last.contains('.') && !word.contains("://")
}

pub fn mentions(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut push = |word: &str| {
        if word.contains("://") {
            return;
        }
        let word = word
            .trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '/' | '.' | ':')));
        let word = word.trim_end_matches(['.', ':']);
        if word.chars().count() >= MIN_MENTION_CHARS && !found.iter().any(|w| w == word) {
            found.push(word.to_string());
        }
    };
    for span in backticked(text) {
        push(span);
        for word in span.split_whitespace() {
            push(word);
        }
    }
    for word in text.split_whitespace() {
        let word = word.trim_matches('`');
        if tokenizer::looks_like_identifier(word) || looks_like_path(word) {
            push(word);
        }
    }
    found
}

pub const MAX_COMMAND_WORDS: usize = 4;

fn command_words(text: &str) -> Vec<&str> {
    backticked(text)
        .into_iter()
        .filter_map(|span| {
            let words: Vec<&str> = span
                .split_whitespace()
                .take_while(|w| !w.starts_with('-'))
                .collect();
            let command = (2..=MAX_COMMAND_WORDS).contains(&words.len())
                && words.iter().all(|w| {
                    w.chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
                });
            command.then(|| words.into_iter().skip(1))
        })
        .flatten()
        .collect()
}

fn is_code_target(unit: &Unit) -> bool {
    !unit.kind().is_documentation()
        && !unit.is_test()
        && !unit.is_vendored()
        && !unit.is_generated()
}

pub fn targets(names: &Names, units: &[Unit], doc: &Unit) -> Vec<UnitId> {
    let mut found: Vec<UnitId> = Vec::new();
    let add = |resolved: &[UnitId], found: &mut Vec<UnitId>| {
        if (1..=MAX_TARGETS_PER_MENTION).contains(&resolved.len()) {
            for unit in resolved {
                if *unit != doc.id && !found.contains(unit) {
                    found.push(*unit);
                }
            }
        }
    };
    for word in command_words(&doc.body) {
        add(names.modules_named(word), &mut found);
    }
    for mention in mentions(&doc.body) {
        let resolved: Vec<UnitId> = if looks_like_path(&mention) {
            names.module_of(&mention).into_iter().collect()
        } else {
            names
                .symbols(&mention)
                .iter()
                .copied()
                .filter(|u| is_code_target(&units[*u as usize]))
                .collect()
        };
        add(&resolved, &mut found);
    }
    found
}

pub fn links(names: &Names, units: &[Unit], existing: &[Edge]) -> Vec<Edge> {
    let known: HashSet<(UnitId, UnitId)> = existing
        .iter()
        .filter(|e| e.relation == Relation::Documents)
        .map(|e| (e.src, e.dst))
        .collect();
    units
        .iter()
        .filter(|u| u.kind().is_documentation())
        .flat_map(|doc| {
            targets(names, units, doc).into_iter().map(move |dst| Edge {
                src: doc.id,
                dst,
                relation: Relation::Documents,
            })
        })
        .filter(|edge| !known.contains(&(edge.src, edge.dst)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: UnitId, name: &str, kind: UnitKind, file: &str, body: &str) -> Unit {
        Unit {
            id,
            name: name.into(),
            qualified_name: name.into(),
            kind: Some(kind),
            file: file.into(),
            body: body.into(),
            ..Unit::default()
        }
    }

    fn names_of(units: &[Unit]) -> HashMap<String, Vec<UnitId>> {
        let mut names: HashMap<String, Vec<UnitId>> = HashMap::new();
        for u in units {
            names
                .entry(tokenizer::name_key(&u.name))
                .or_default()
                .push(u.id);
        }
        names
    }

    #[test]
    fn mentions_come_from_backticks_identifiers_and_paths() {
        let found = mentions("Run `amafu install`, which opens an InstallSession via `MaasClient::new` in crates/installer/src/maas.rs. See https://example.com/a.html");
        for expected in [
            "amafu install",
            "amafu",
            "install",
            "InstallSession",
            "MaasClient::new",
            "crates/installer/src/maas.rs",
        ] {
            assert!(
                found.iter().any(|f| f == expected),
                "{expected} missing from {found:?}"
            );
        }
        assert!(!found.iter().any(|f| f.contains("example.com")));
    }

    #[test]
    fn documentation_links_to_the_rare_code_it_names() {
        let mut units = vec![
            unit(0, "Install with MAAS", UnitKind::Doc, "README.md", "Uses `MaasClient` and `new` from crates/installer/src/maas.rs, unlike `planned_command`."),
            unit(1, "MaasClient", UnitKind::Class, "crates/installer/src/maas.rs", ""),
            unit(2, "crates/installer/src/maas.rs", UnitKind::Module, "crates/installer/src/maas.rs", ""),
            unit(3, "Other docs", UnitKind::Doc, "docs/other.md", "MaasClient"),
        ];
        for i in 0..5 {
            units.push(unit(
                4 + i,
                "new",
                UnitKind::Function,
                &format!("src/f{i}.rs"),
                "",
            ));
        }
        let names = names_of(&units);
        let index = Names::new(&names, &units);
        let linked = targets(&index, &units, &units[0]);
        assert_eq!(
            linked,
            vec![1, 2],
            "common names and unknown commands are skipped, docs are never targets"
        );
        let edges = links(&index, &units, &[]);
        assert!(edges.iter().all(|e| e.relation == Relation::Documents));
        assert!(edges.iter().any(|e| e.src == 3 && e.dst == 1));
        assert!(!edges.iter().any(|e| e.dst == 0 || e.dst == 3));
    }

    #[test]
    fn existing_documentation_edges_are_not_duplicated() {
        let units = vec![
            unit(0, "Guide", UnitKind::Doc, "README.md", "`MaasClient`"),
            unit(1, "MaasClient", UnitKind::Class, "maas.rs", ""),
        ];
        let names = names_of(&units);
        let index = Names::new(&names, &units);
        let existing = vec![Edge {
            src: 0,
            dst: 1,
            relation: Relation::Documents,
        }];
        assert!(links(&index, &units, &existing).is_empty());
    }

    #[test]
    fn commands_lead_to_the_modules_named_after_their_subcommands() {
        let units = vec![
            unit(
                0,
                "Installing",
                UnitKind::Doc,
                "docs/install.md",
                "Run `amafu install` from the operator machine.",
            ),
            unit(
                1,
                "crates/command-line/src/install.rs",
                UnitKind::Module,
                "crates/command-line/src/install.rs",
                "",
            ),
            unit(
                2,
                "crates/command-line/src/upgrade.rs",
                UnitKind::Module,
                "crates/command-line/src/upgrade.rs",
                "",
            ),
            unit(
                3,
                "docs/install.md",
                UnitKind::Module,
                "docs/install.md",
                "",
            ),
        ];
        let mut units = units;
        units[3].kind = Some(UnitKind::Doc);
        let names = names_of(&units);
        let index = Names::new(&names, &units);
        assert_eq!(targets(&index, &units, &units[0]), vec![1]);
        assert_eq!(command_words("`amafu install --tag x`"), vec!["install"]);
        assert!(
            command_words("`install`").is_empty(),
            "a single word is not a command"
        );
    }
}
