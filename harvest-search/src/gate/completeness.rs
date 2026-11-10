use std::collections::HashSet;

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::{Gap, UnitId};
use crate::gate::{SufficiencyCheck, Verdict};
use crate::index::corpus::{Relation, UnitKind};
use crate::index::graph::Depth;
use crate::index::graph::Direction;
use crate::index::tokenizer::tokens;

const MIN_FAMILY: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    pub root: String,
    pub found: usize,
    pub total: usize,
}

impl Coverage {
    pub fn ratio(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.found as f32 / self.total as f32
        }
    }
}

fn family(ctx: &SearchContext, root: UnitId) -> Vec<UnitId> {
    ctx.index
        .graph
        .subclasses(root, Depth::Transitive)
        .into_iter()
        .filter(|u| !ctx.index.unit(*u).is_test())
        .collect()
}

fn root_class(ctx: &SearchContext) -> Option<UnitId> {
    let index = &ctx.index;
    let is_class = |u: &UnitId| index.unit(*u).kind() == UnitKind::Class;
    for identifier in &ctx.question.identifiers {
        if let Some(root) = index
            .find_by_name(identifier)
            .iter()
            .copied()
            .filter(is_class)
            .max_by_key(|u| family(ctx, *u).len())
        {
            if family(ctx, root).len() >= MIN_FAMILY {
                return Some(root);
            }
        }
    }
    let vocabulary: HashSet<String> = ctx.question.vocabulary().into_iter().collect();
    let mut related: Vec<UnitId> = ctx
        .evidence_units()
        .into_iter()
        .filter(is_class)
        .flat_map(|u| {
            let mut chain = vec![u];
            chain.extend(index.graph.ancestors(u));
            chain
        })
        .filter(|u| {
            tokens(&index.unit(*u).name)
                .iter()
                .any(|t| vocabulary.contains(t) || vocabulary.contains(t.trim_end_matches('s')))
        })
        .collect();
    related.sort_unstable();
    related.dedup();
    related
        .into_iter()
        .map(|u| (family(ctx, u).len(), u))
        .filter(|(size, _)| *size >= MIN_FAMILY)
        .max()
        .map(|(_, u)| u)
}

const CALLER_CUES: &[&str] = &[
    "calls ",
    "callers",
    "called by",
    "who uses",
    "used by",
    "invokes",
];

fn named_unit(ctx: &SearchContext) -> Option<UnitId> {
    ctx.question.identifiers.iter().find_map(|identifier| {
        let found = ctx.index.find_by_name(identifier);
        (found.len() == 1).then(|| found[0])
    })
}

pub fn caller_coverage(ctx: &SearchContext) -> Option<Coverage> {
    let lower = ctx.query.text.to_lowercase();
    if !CALLER_CUES.iter().any(|cue| lower.contains(cue)) {
        return None;
    }
    let target = named_unit(ctx)?;
    let callers: Vec<UnitId> = ctx
        .index
        .graph
        .neighbors(target, Direction::In, &[Relation::Calls])
        .collect();
    if callers.is_empty() {
        return None;
    }
    let found = callers.iter().filter(|u| ctx.has_unit(**u)).count();
    Some(Coverage {
        root: format!("callers of {}", ctx.index.unit(target).qualified_name),
        found,
        total: callers.len(),
    })
}

pub fn coverage(ctx: &SearchContext) -> Option<Coverage> {
    if let Some(callers) = caller_coverage(ctx) {
        return Some(callers);
    }
    if !ctx.question.wants_list {
        return None;
    }
    let root = root_class(ctx)?;
    let members = family(ctx, root);
    let found = members.iter().filter(|u| ctx.has_unit(**u)).count();
    Some(Coverage {
        root: ctx.index.unit(root).qualified_name.clone(),
        found,
        total: members.len(),
    })
}

pub struct ListCompleteness {
    pub threshold: f32,
}

impl Default for ListCompleteness {
    fn default() -> Self {
        Self { threshold: 0.9 }
    }
}

#[async_trait]
impl SufficiencyCheck for ListCompleteness {
    fn name(&self) -> &'static str {
        "list_completeness"
    }

    async fn check(&self, ctx: &SearchContext) -> Verdict {
        match coverage(ctx) {
            Some(c) if c.ratio() < self.threshold => {
                let of = if c.root.starts_with("callers of ") {
                    c.root.clone()
                } else {
                    format!("subclasses of {}", c.root)
                };
                Verdict::Insufficient(vec![Gap::IncompleteList {
                    of,
                    found: c.found,
                    total: c.total,
                }])
            }
            _ => Verdict::Undecided,
        }
    }
}
