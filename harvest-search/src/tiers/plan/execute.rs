use std::collections::HashSet;

use crate::domain::{Evidence, EvidenceKey, SearchError, Source, UnitId};
use crate::index::corpus::{Relation, UnitKind};
use crate::index::graph::Direction;
use crate::index::tokenizer::query_tokens;
use crate::index::IndexSnapshot;
use crate::retrieve::{Candidate, Terms};
use crate::tiers::plan::step::{Step, StepRef};

const SEARCH_RESULTS: usize = 8;
const MAX_REF_UNITS: usize = 200;

#[derive(Clone, Debug, Default)]
pub struct StepOutput {
    pub units: Vec<UnitId>,
    pub table: Option<Evidence>,
    pub read: Vec<UnitId>,
    pub summary: String,
}

#[derive(Clone, Debug, Default)]
pub struct Executed {
    pub outputs: Vec<StepOutput>,
    pub candidates: Vec<Candidate>,
    pub tables: Vec<Evidence>,
}

pub fn capability_value(source: &str, name: &str) -> Option<String> {
    let mut body_indent: Option<usize> = None;
    for line in source.lines().skip(1) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent != *body_indent.get_or_insert(indent) {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(name) else {
            continue;
        };
        if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = rest.trim_start();
        let rest = match rest.strip_prefix(':') {
            Some(annotated) => match annotated.find('=') {
                Some(i) => &annotated[i..],
                None => continue,
            },
            None => rest,
        };
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        if value.starts_with('=') {
            continue;
        }
        let value = value.split(" #").next().unwrap_or(value).trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn table(name: String, lines: Vec<String>, covers: Vec<EvidenceKey>, source: Source) -> Evidence {
    Evidence {
        key: EvidenceKey::Symbol {
            file: String::new(),
            name: name.clone(),
        },
        unit: None,
        label: name,
        start_line: 0,
        end_line: 0,
        text: lines.join("\n"),
        score: 0.0,
        found_by: source,
        covers,
    }
}

pub struct Executor<'a> {
    index: &'a IndexSnapshot,
    source: Source,
}

impl<'a> Executor<'a> {
    pub fn new(index: &'a IndexSnapshot, source: Source) -> Self {
        Self { index, source }
    }

    fn resolve(
        &self,
        reference: &StepRef,
        outputs: &[StepOutput],
    ) -> Result<Vec<UnitId>, SearchError> {
        let mut units = match reference {
            StepRef::Index(i) => outputs
                .get(*i)
                .map(|o| o.units.clone())
                .ok_or_else(|| SearchError::Plan(format!("step {i} does not exist yet")))?,
            StepRef::Name(name) => self.index.find_by_name(name).to_vec(),
        };
        units.truncate(MAX_REF_UNITS);
        Ok(units)
    }

    fn location(&self, unit: UnitId) -> String {
        let u = self.index.unit(unit);
        format!("{} ({}:{})", u.qualified_name, u.file, u.start_line)
    }

    fn classes(&self, units: &[UnitId]) -> Vec<UnitId> {
        units
            .iter()
            .copied()
            .filter(|u| self.index.unit(*u).kind() == UnitKind::Class)
            .collect()
    }

    fn search(&self, terms: &str) -> Vec<UnitId> {
        let terms = Terms::from_question(terms);
        let mut found: Vec<UnitId> = terms
            .identifiers()
            .iter()
            .flat_map(|n| self.index.find_by_name(n).to_vec())
            .collect();
        for (unit, _) in self.index.bm25.search(terms.weighted(), SEARCH_RESULTS) {
            if !found.contains(&unit) {
                found.push(unit);
            }
        }
        found.truncate(SEARCH_RESULTS);
        found
    }

    fn method_of(&self, class: UnitId, name: &str) -> Option<(UnitId, UnitId)> {
        std::iter::once(class)
            .chain(self.index.graph.ancestors(class))
            .find_map(|owner| {
                self.index
                    .graph
                    .neighbors(owner, Direction::Out, &[Relation::Contains])
                    .find(|m| self.index.unit(*m).name == name)
                    .map(|m| (owner, m))
            })
    }

    pub fn step(&self, step: &Step, outputs: &[StepOutput]) -> Result<StepOutput, SearchError> {
        let index = self.index;
        Ok(match step {
            Step::Search { terms } => {
                let units = self.search(terms);
                StepOutput {
                    summary: format!("search {terms:?}: {}", self.names(&units)),
                    units,
                    ..StepOutput::default()
                }
            }
            Step::SubclassesOf { class, depth } => {
                let roots = self.classes(index.find_by_name(class));
                let mut units: Vec<UnitId> = Vec::new();
                for root in roots {
                    for sub in index.graph.subclasses(root, (*depth).into()) {
                        if !index.unit(sub).is_test() && !units.contains(&sub) {
                            units.push(sub);
                        }
                    }
                }
                let lines = units
                    .iter()
                    .map(|u| {
                        let unit = index.unit(*u);
                        format!("{} extends {}", self.location(*u), unit.bases.join(", "))
                    })
                    .collect();
                let covers = units.iter().map(|u| index.unit(*u).key()).collect();
                StepOutput {
                    summary: format!("{} subclasses of {class}", units.len()),
                    table: Some(table(
                        format!("subclasses of {class}"),
                        lines,
                        covers,
                        self.source,
                    )),
                    units,
                    ..StepOutput::default()
                }
            }
            Step::Capability { of, name } => {
                let classes = self.classes(&self.resolve(of, outputs)?);
                let mut lines = Vec::new();
                let mut covers = Vec::new();
                for class in &classes {
                    let chain: Vec<UnitId> = std::iter::once(*class)
                        .chain(index.graph.ancestors(*class))
                        .collect();
                    let found = chain.iter().find_map(|c| {
                        capability_value(&index.unit(*c).body, name).map(|v| (*c, v))
                    });
                    covers.push(index.unit(*class).key());
                    lines.push(match found {
                        Some((owner, value)) if owner == *class => {
                            format!("{}: {name} = {value}", self.location(*class))
                        }
                        Some((owner, value)) => {
                            covers.push(index.unit(owner).key());
                            format!(
                                "{}: {name} = {value} (inherited from {})",
                                self.location(*class),
                                index.unit(owner).qualified_name
                            )
                        }
                        None => format!("{}: {name} is not defined", self.location(*class)),
                    });
                }
                covers.sort();
                covers.dedup();
                StepOutput {
                    summary: format!("{name} resolved for {} classes", classes.len()),
                    table: Some(table(
                        format!("capability {name}"),
                        lines,
                        covers,
                        self.source,
                    )),
                    units: classes,
                    ..StepOutput::default()
                }
            }
            Step::Method { of, name } => {
                let mut units = Vec::new();
                let mut notes = Vec::new();
                for class in self.classes(&self.resolve(of, outputs)?) {
                    match self.method_of(class, name) {
                        Some((owner, method)) => {
                            if owner != class {
                                notes.push(format!(
                                    "{} inherits {name} from {}",
                                    index.unit(class).qualified_name,
                                    index.unit(owner).qualified_name
                                ));
                            }
                            if !units.contains(&method) {
                                units.push(method);
                            }
                        }
                        None => notes.push(format!(
                            "{} has no {name}",
                            index.unit(class).qualified_name
                        )),
                    }
                }
                StepOutput {
                    summary: format!(
                        "{} implementations of {name}. {}",
                        units.len(),
                        notes.join("; ")
                    ),
                    read: units.clone(),
                    units,
                    ..StepOutput::default()
                }
            }
            Step::Callers { of } | Step::Callees { of } => {
                let direction = if matches!(step, Step::Callers { .. }) {
                    Direction::In
                } else {
                    Direction::Out
                };
                let mut units: Vec<UnitId> = Vec::new();
                for unit in self.resolve(of, outputs)? {
                    for next in index.graph.neighbors(unit, direction, &[Relation::Calls]) {
                        if !units.contains(&next) {
                            units.push(next);
                        }
                    }
                }
                let label = if direction == Direction::In {
                    "callers"
                } else {
                    "callees"
                };
                let lines = units.iter().map(|u| self.location(*u)).collect();
                let covers = units.iter().map(|u| index.unit(*u).key()).collect();
                StepOutput {
                    summary: format!("{} {label}: {}", units.len(), self.names(&units)),
                    table: Some(table(
                        format!("{label} of {}", ref_label(of)),
                        lines,
                        covers,
                        self.source,
                    )),
                    units,
                    ..StepOutput::default()
                }
            }
            Step::ReadsConfig { key } => {
                let mut units: Vec<UnitId> = index
                    .find_by_name(key)
                    .iter()
                    .flat_map(|c| {
                        index
                            .graph
                            .neighbors(*c, Direction::In, &[Relation::ReadsConfig])
                    })
                    .collect();
                let wanted: HashSet<String> = query_tokens(key).into_iter().collect();
                let terms: Vec<(String, f32)> = wanted.iter().map(|t| (t.clone(), 1.0)).collect();
                for (unit, _) in index.bm25.search(&terms, SEARCH_RESULTS * 2) {
                    if index.unit(unit).body.contains(key.as_str()) && !units.contains(&unit) {
                        units.push(unit);
                    }
                }
                units.truncate(SEARCH_RESULTS);
                StepOutput {
                    summary: format!("{} readers of {key}: {}", units.len(), self.names(&units)),
                    read: units.clone(),
                    units,
                    ..StepOutput::default()
                }
            }
            Step::Read { of, budget_tokens } => {
                let mut units = self.resolve(of, outputs)?;
                let mut spent = 0usize;
                units.retain(|u| {
                    let fits = spent < *budget_tokens as usize;
                    spent += index.unit(*u).body.len() / 4;
                    fits
                });
                StepOutput {
                    summary: format!("read {}", self.names(&units)),
                    read: units.clone(),
                    units,
                    ..StepOutput::default()
                }
            }
        })
    }

    fn names(&self, units: &[UnitId]) -> String {
        let names: Vec<&str> = units
            .iter()
            .take(12)
            .map(|u| self.index.unit(*u).qualified_name.as_str())
            .collect();
        let more = units.len().saturating_sub(names.len());
        if more > 0 {
            format!("{} and {more} more", names.join(", "))
        } else {
            names.join(", ")
        }
    }

    pub fn run(&self, steps: &[Step], prior: &[StepOutput]) -> Executed {
        let mut outputs: Vec<StepOutput> = prior.to_vec();
        let mut executed = Executed::default();
        for step in steps {
            let output = self.step(step, &outputs).unwrap_or_else(|e| StepOutput {
                summary: format!("failed: {e}"),
                ..StepOutput::default()
            });
            for unit in &output.read {
                executed
                    .candidates
                    .push(Candidate::pinned(*unit, self.source));
            }
            for (rank, unit) in output.units.iter().enumerate() {
                if !output.read.contains(unit) {
                    executed.candidates.push(Candidate {
                        unit: *unit,
                        score: 1.0 / (rank as f32 + 2.0),
                        sources: vec![self.source],
                        ..Candidate::default()
                    });
                }
            }
            if let Some(table) = &output.table {
                executed.tables.push(table.clone());
            }
            outputs.push(output);
        }
        executed.outputs = outputs.split_off(prior.len());
        executed
    }
}

fn ref_label(reference: &StepRef) -> String {
    match reference {
        StepRef::Index(i) => format!("step {i}"),
        StepRef::Name(name) => name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::{Corpus, Unit};
    use crate::index::vectors::HashingEmbedder;
    use crate::tiers::plan::step::Depth;

    #[test]
    fn capability_values_come_from_class_level_assignments_only() {
        let source = "class LVM(Driver):\n    supports_multiattach = True  # yes\n    other: bool = False\n\n    def f(self):\n        supports_multiattach = False\n";
        assert_eq!(
            capability_value(source, "supports_multiattach").as_deref(),
            Some("True")
        );
        assert_eq!(capability_value(source, "other").as_deref(), Some("False"));
        assert_eq!(capability_value(source, "supports"), None);
        assert_eq!(capability_value("class A:\n    x == 1\n", "x"), None);
    }

    async fn drivers() -> IndexSnapshot {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        let class = |name: &str, body: &str, bases: &[&str]| Unit {
            name: name.into(),
            kind: Some(UnitKind::Class),
            file: format!("drivers/{}.py", name.to_lowercase()),
            body: body.into(),
            bases: bases.iter().map(|b| b.to_string()).collect(),
            ..Unit::default()
        };
        corpus.push_unit(class(
            "VolumeDriver",
            "class VolumeDriver:\n    supports_multiattach = False\n",
            &[],
        ));
        corpus.push_unit(class(
            "LVM",
            "class LVM(VolumeDriver):\n    supports_multiattach = True\n",
            &["VolumeDriver"],
        ));
        corpus.push_unit(class(
            "NFS",
            "class NFS(VolumeDriver):\n    pass\n",
            &["VolumeDriver"],
        ));
        corpus.push_unit(Unit {
            name: "initialize_connection".into(),
            qualified_name: "VolumeDriver.initialize_connection".into(),
            parent: Some("VolumeDriver".into()),
            kind: Some(UnitKind::Method),
            file: "drivers/volumedriver.py".into(),
            body: "def initialize_connection(self): ...".into(),
            ..Unit::default()
        });
        corpus.link(1, 0, Relation::Inherits);
        corpus.link(2, 0, Relation::Inherits);
        corpus.link(0, 3, Relation::Contains);
        IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_capability_plan_resolves_values_through_inheritance() {
        let index = drivers().await;
        let executor = Executor::new(&index, Source::Plan);
        let executed = executor.run(
            &[
                Step::SubclassesOf {
                    class: "VolumeDriver".into(),
                    depth: Depth::Transitive,
                },
                Step::Capability {
                    of: StepRef::Index(0),
                    name: "supports_multiattach".into(),
                },
                Step::Method {
                    of: StepRef::Index(0),
                    name: "initialize_connection".into(),
                },
            ],
            &[],
        );
        assert_eq!(executed.outputs[0].units, vec![1, 2]);
        let matrix = &executed.tables[1].text;
        assert!(
            matrix.contains("LVM (drivers/lvm.py:0): supports_multiattach = True"),
            "{matrix}"
        );
        assert!(matrix.contains("NFS (drivers/nfs.py:0): supports_multiattach = False (inherited from VolumeDriver)"), "{matrix}");
        assert_eq!(executed.tables[1].covers.len(), 3);
        assert_eq!(executed.outputs[2].units, vec![3]);
        assert!(executed.outputs[2]
            .summary
            .contains("LVM inherits initialize_connection from VolumeDriver"));
        assert!(executed.candidates.iter().any(|c| c.unit == 3 && c.pinned));
    }

    #[tokio::test]
    async fn references_to_missing_steps_fail_only_that_step() {
        let index = drivers().await;
        let executed = Executor::new(&index, Source::Step).run(
            &[
                Step::Read {
                    of: StepRef::Index(5),
                    budget_tokens: 100,
                },
                Step::Search {
                    terms: "LVM driver".into(),
                },
            ],
            &[],
        );
        assert!(executed.outputs[0].summary.starts_with("failed"));
        assert_eq!(executed.outputs[1].units[0], 1);
    }
}
