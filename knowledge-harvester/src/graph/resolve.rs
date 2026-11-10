use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Label {
    Class,
    Function,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SymbolRecord {
    pub id: i64,
    pub file: String,
    pub label: Label,
    pub name: String,
    pub impl_type: Option<String>,
    pub bases: Vec<String>,
    pub traits: Vec<String>,
    pub embeds: Vec<String>,
    pub uses: Vec<String>,
    pub decorators: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallRecord {
    pub caller: i64,
    pub callee: String,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    Raises,
    Registers,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceRecord {
    pub from: i64,
    pub target: String,
    pub kind: RefKind,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResolvedEdge {
    pub src: i64,
    pub dst: i64,
    pub relation: &'static str,
    pub line: Option<u32>,
}

pub fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.starts_with("tests/")
        || p.starts_with("test/")
        || p.contains("/tests/")
        || p.contains("/test/")
        || p.contains("/testing/")
        || p.rsplit('/').next().is_some_and(|f| {
            f.starts_with("test_")
                || f.contains("_test.")
                || f.contains(".test.")
                || f.contains(".spec.")
        })
}

fn last_segment(name: &str) -> &str {
    name.rsplit(['.', ':']).next().unwrap_or(name).trim()
}

fn module_prefixes(target: &str) -> Vec<String> {
    let path = target.trim_start_matches('.').replace(['.', ':'], "/");
    if path.is_empty() {
        return Vec::new();
    }
    vec![path]
}

pub struct Resolver<'a> {
    symbols: &'a [SymbolRecord],
    by_id: HashMap<i64, usize>,
    functions: HashMap<&'a str, Vec<usize>>,
    classes: HashMap<&'a str, Vec<usize>>,
    members: HashMap<(&'a str, &'a str), Vec<usize>>,
    imports: HashMap<&'a str, Vec<String>>,
    parents: HashMap<usize, Vec<usize>>,
}

impl<'a> Resolver<'a> {
    pub fn new(symbols: &'a [SymbolRecord], imports: &'a HashMap<String, Vec<String>>) -> Self {
        let mut resolver = Self {
            symbols,
            by_id: HashMap::new(),
            functions: HashMap::new(),
            classes: HashMap::new(),
            members: HashMap::new(),
            imports: imports
                .iter()
                .map(|(file, targets)| {
                    (
                        file.as_str(),
                        targets.iter().flat_map(|t| module_prefixes(t)).collect(),
                    )
                })
                .collect(),
            parents: HashMap::new(),
        };
        for (i, s) in symbols.iter().enumerate() {
            resolver.by_id.insert(s.id, i);
            match s.label {
                Label::Function => {
                    resolver
                        .functions
                        .entry(s.name.as_str())
                        .or_default()
                        .push(i);
                    if let Some(owner) = &s.impl_type {
                        resolver
                            .members
                            .entry((owner.as_str(), s.name.as_str()))
                            .or_default()
                            .push(i);
                    }
                }
                Label::Class => resolver.classes.entry(s.name.as_str()).or_default().push(i),
            }
        }
        for (i, s) in symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.label == Label::Class)
        {
            let parents: Vec<usize> = s
                .bases
                .iter()
                .chain(&s.traits)
                .filter_map(|b| resolver.class_named(s, b))
                .filter(|p| *p != i)
                .collect();
            if !parents.is_empty() {
                resolver.parents.insert(i, parents);
            }
        }
        resolver
    }

    fn imported(&self, from: &SymbolRecord, candidate: &SymbolRecord) -> bool {
        self.imports
            .get(from.file.as_str())
            .is_some_and(|prefixes| {
                prefixes
                    .iter()
                    .any(|p| candidate.file.starts_with(p.as_str()))
            })
    }

    fn pick(&self, from: &SymbolRecord, candidates: &[usize]) -> Option<usize> {
        match candidates {
            [] => None,
            [only] => Some(*only),
            many => {
                let same_file: Vec<usize> = many
                    .iter()
                    .copied()
                    .filter(|c| self.symbols[*c].file == from.file)
                    .collect();
                if same_file.len() == 1 {
                    return Some(same_file[0]);
                }
                let imported: Vec<usize> = many
                    .iter()
                    .copied()
                    .filter(|c| self.imported(from, &self.symbols[*c]))
                    .collect();
                if imported.len() == 1 {
                    return Some(imported[0]);
                }
                let non_test: Vec<usize> = many
                    .iter()
                    .copied()
                    .filter(|c| !is_test_path(&self.symbols[*c].file))
                    .collect();
                (non_test.len() == 1).then(|| non_test[0])
            }
        }
    }

    pub fn class_named(&self, from: &SymbolRecord, name: &str) -> Option<usize> {
        let candidates = self.classes.get(last_segment(name))?;
        self.pick(from, candidates)
    }

    pub fn ancestors(&self, class: usize) -> Vec<usize> {
        let mut seen = HashSet::from([class]);
        let mut order = Vec::new();
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            for p in self.parents.get(&c).into_iter().flatten() {
                if seen.insert(*p) {
                    order.push(*p);
                    queue.push_back(*p);
                }
            }
        }
        order
    }

    fn owner_class(&self, symbol: &SymbolRecord) -> Option<usize> {
        let owner = symbol.impl_type.as_deref()?;
        self.classes
            .get(owner)?
            .iter()
            .copied()
            .find(|c| self.symbols[*c].file == symbol.file)
            .or_else(|| self.class_named(symbol, owner))
    }

    fn base_of_all(&self, candidates: &[usize]) -> Option<usize> {
        let owners: Vec<(usize, Option<usize>)> = candidates
            .iter()
            .map(|c| (*c, self.owner_class(&self.symbols[*c])))
            .collect();
        owners.iter().find_map(|(candidate, owner)| {
            let owner = (*owner)?;
            let covers_all = owners.iter().all(|(other, other_owner)| {
                other == candidate
                    || other_owner.is_some_and(|o| self.ancestors(o).contains(&owner))
            });
            covers_all.then_some(*candidate)
        })
    }

    pub fn callee(&self, caller: &SymbolRecord, name: &str) -> Option<(usize, &'static str)> {
        let name = last_segment(name);
        if let Some(class) = self.owner_class(caller) {
            for owner in std::iter::once(class).chain(self.ancestors(class)) {
                let owner_name = self.symbols[owner].name.as_str();
                if let Some(found) = self.members.get(&(owner_name, name)) {
                    if let Some(found) = self.pick(caller, found) {
                        return Some((found, "CALLS"));
                    }
                }
            }
        }
        if let Some(candidates) = self.functions.get(name) {
            if let Some(found) = self.pick(caller, candidates) {
                return Some((found, "CALLS"));
            }
            if let Some(base) = self.base_of_all(candidates) {
                return Some((base, "CALLS"));
            }
        }
        if name.starts_with(char::is_uppercase) {
            if let Some(class) = self.class_named(caller, name) {
                return Some((class, "INSTANTIATES"));
            }
        }
        None
    }

    fn function_named(&self, from: &SymbolRecord, name: &str) -> Option<usize> {
        self.pick(from, self.functions.get(last_segment(name))?)
    }

    pub fn edges(&self, calls: &[CallRecord], references: &[ReferenceRecord]) -> Vec<ResolvedEdge> {
        let mut edges: HashSet<ResolvedEdge> = HashSet::new();
        let id = |i: usize| self.symbols[i].id;
        for (i, s) in self
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.label == Label::Class)
        {
            for (names, relation) in [
                (&s.bases, "INHERITS"),
                (&s.traits, "IMPLEMENTS"),
                (&s.embeds, "EMBEDS"),
                (&s.uses, "USES"),
            ] {
                for name in names {
                    if let Some(target) = self.class_named(s, name).filter(|t| *t != i) {
                        edges.insert(ResolvedEdge {
                            src: s.id,
                            dst: id(target),
                            relation,
                            line: None,
                        });
                    }
                }
            }
            for decorator in &s.decorators {
                if let Some(registrar) = self.function_named(s, decorator.trim_start_matches('@')) {
                    edges.insert(ResolvedEdge {
                        src: id(registrar),
                        dst: s.id,
                        relation: "REGISTERS",
                        line: None,
                    });
                }
            }
        }
        for (i, s) in self
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.label == Label::Function)
        {
            let Some(class) = self.owner_class(s) else {
                continue;
            };
            for ancestor in self.ancestors(class) {
                let owner = self.symbols[ancestor].name.as_str();
                if let Some(base) = self
                    .members
                    .get(&(owner, s.name.as_str()))
                    .and_then(|found| found.iter().copied().find(|m| *m != i))
                {
                    edges.insert(ResolvedEdge {
                        src: s.id,
                        dst: id(base),
                        relation: "OVERRIDES",
                        line: None,
                    });
                    break;
                }
            }
        }
        for call in calls {
            let Some(caller) = self.by_id.get(&call.caller).map(|i| &self.symbols[*i]) else {
                continue;
            };
            if let Some((target, relation)) = self.callee(caller, &call.callee) {
                if id(target) == caller.id {
                    continue;
                }
                edges.insert(ResolvedEdge {
                    src: caller.id,
                    dst: id(target),
                    relation,
                    line: Some(call.line),
                });
                if is_test_path(&caller.file) && !is_test_path(&self.symbols[target].file) {
                    edges.insert(ResolvedEdge {
                        src: caller.id,
                        dst: id(target),
                        relation: "TESTS",
                        line: None,
                    });
                }
            }
        }
        for reference in references {
            let Some(from) = self.by_id.get(&reference.from).map(|i| &self.symbols[*i]) else {
                continue;
            };
            let (target, relation) = match reference.kind {
                RefKind::Raises => (self.class_named(from, &reference.target), "RAISES"),
                RefKind::Registers => (self.function_named(from, &reference.target), "REGISTERS"),
            };
            if let Some(target) = target {
                let (src, dst) = match reference.kind {
                    RefKind::Raises => (from.id, id(target)),
                    RefKind::Registers => (id(target), from.id),
                };
                if src != dst {
                    edges.insert(ResolvedEdge {
                        src,
                        dst,
                        relation,
                        line: Some(reference.line),
                    });
                }
            }
        }
        let mut sorted: Vec<ResolvedEdge> = edges.into_iter().collect();
        sorted.sort();
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(id: i64, file: &str, name: &str, bases: &[&str]) -> SymbolRecord {
        SymbolRecord {
            id,
            file: file.into(),
            label: Label::Class,
            name: name.into(),
            impl_type: None,
            bases: bases.iter().map(|b| b.to_string()).collect(),
            traits: vec![],
            embeds: vec![],
            uses: vec![],
            decorators: vec![],
        }
    }

    fn method(id: i64, file: &str, owner: Option<&str>, name: &str) -> SymbolRecord {
        SymbolRecord {
            id,
            file: file.into(),
            label: Label::Function,
            name: name.into(),
            impl_type: owner.map(str::to_string),
            bases: vec![],
            traits: vec![],
            embeds: vec![],
            uses: vec![],
            decorators: vec![],
        }
    }

    fn fixture() -> Vec<SymbolRecord> {
        vec![
            class(1, "cinder/volume/driver.py", "VolumeDriver", &[]),
            method(
                2,
                "cinder/volume/driver.py",
                Some("VolumeDriver"),
                "create_volume",
            ),
            class(
                3,
                "cinder/volume/drivers/lvm.py",
                "LVMDriver",
                &["driver.VolumeDriver"],
            ),
            method(
                4,
                "cinder/volume/drivers/lvm.py",
                Some("LVMDriver"),
                "create_volume",
            ),
            class(
                5,
                "cinder/volume/drivers/nfs.py",
                "NfsDriver",
                &["VolumeDriver"],
            ),
            method(
                6,
                "cinder/volume/drivers/nfs.py",
                Some("NfsDriver"),
                "create_volume",
            ),
            method(
                7,
                "cinder/volume/drivers/nfs.py",
                Some("NfsDriver"),
                "_mount",
            ),
            method(
                8,
                "cinder/volume/manager.py",
                Some("VolumeManager"),
                "create",
            ),
            class(9, "cinder/volume/manager.py", "VolumeManager", &[]),
            method(10, "cinder/tests/test_lvm.py", None, "test_create"),
            class(11, "cinder/exception.py", "VolumeBusy", &[]),
            class(12, "other/driver.py", "VolumeDriver", &[]),
            method(
                13,
                "cinder/volume/drivers/nfs.py",
                Some("NfsDriver"),
                "__init__",
            ),
            method(
                14,
                "cinder/volume/drivers/lvm.py",
                Some("LVMDriver"),
                "__init__",
            ),
        ]
    }

    fn imports() -> HashMap<String, Vec<String>> {
        HashMap::from([
            (
                "cinder/volume/drivers/lvm.py".to_string(),
                vec!["cinder.volume".to_string()],
            ),
            (
                "cinder/volume/drivers/nfs.py".to_string(),
                vec!["cinder.volume.driver".to_string()],
            ),
        ])
    }

    fn has(edges: &[ResolvedEdge], src: i64, dst: i64, relation: &str) -> bool {
        edges
            .iter()
            .any(|e| e.src == src && e.dst == dst && e.relation == relation)
    }

    #[test]
    fn inheritance_uses_imports_to_pick_between_same_named_classes() {
        let symbols = fixture();
        let imports = imports();
        let resolver = Resolver::new(&symbols, &imports);
        let edges = resolver.edges(&[], &[]);
        assert!(has(&edges, 3, 1, "INHERITS"));
        assert!(has(&edges, 5, 1, "INHERITS"));
        assert!(!has(&edges, 3, 12, "INHERITS"));
    }

    #[test]
    fn methods_override_the_nearest_ancestor_method() {
        let symbols = fixture();
        let imports = imports();
        let edges = Resolver::new(&symbols, &imports).edges(&[], &[]);
        assert!(has(&edges, 4, 2, "OVERRIDES"));
        assert!(has(&edges, 6, 2, "OVERRIDES"));
        assert!(!edges
            .iter()
            .any(|e| e.src == 7 && e.relation == "OVERRIDES"));
    }

    #[test]
    fn calls_prefer_the_own_class_then_resolve_polymorphic_calls_to_the_base() {
        let symbols = fixture();
        let imports = imports();
        let resolver = Resolver::new(&symbols, &imports);
        let calls = vec![
            CallRecord {
                caller: 6,
                callee: "_mount".into(),
                line: 3,
            },
            CallRecord {
                caller: 8,
                callee: "create_volume".into(),
                line: 9,
            },
            CallRecord {
                caller: 8,
                callee: "VolumeBusy".into(),
                line: 10,
            },
            CallRecord {
                caller: 10,
                callee: "create_volume".into(),
                line: 2,
            },
            CallRecord {
                caller: 13,
                callee: "__init__".into(),
                line: 1,
            },
        ];
        let edges = resolver.edges(&calls, &[]);
        assert!(has(&edges, 6, 7, "CALLS"));
        assert!(has(&edges, 8, 2, "CALLS"), "{edges:?}");
        assert!(has(&edges, 8, 11, "INSTANTIATES"));
        assert!(has(&edges, 10, 2, "TESTS"));
        assert!(
            !edges.iter().any(|e| e.src == 13 && e.relation == "CALLS"),
            "{edges:?}"
        );
    }

    #[test]
    fn raises_and_registrations_become_edges() {
        let mut symbols = fixture();
        symbols.push(method(20, "cinder/interface.py", None, "volumedriver"));
        symbols[2].decorators = vec!["@interface.volumedriver".into()];
        let imports = imports();
        let resolver = Resolver::new(&symbols, &imports);
        let edges = resolver.edges(
            &[],
            &[ReferenceRecord {
                from: 8,
                target: "exception.VolumeBusy".into(),
                kind: RefKind::Raises,
                line: 4,
            }],
        );
        assert!(has(&edges, 8, 11, "RAISES"));
        assert!(has(&edges, 20, 3, "REGISTERS"));
    }

    #[test]
    fn test_paths_are_recognised() {
        assert!(is_test_path("cinder/tests/unit/test_x.py"));
        assert!(is_test_path("src/foo_test.go"));
        assert!(is_test_path("web/app.spec.ts"));
        assert!(!is_test_path("cinder/volume/manager.py"));
    }
}
