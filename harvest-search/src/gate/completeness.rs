use std::collections::HashSet;

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::{Gap, UnitId};
use crate::gate::{SufficiencyCheck, Verdict};
use crate::index::corpus::UnitKind;
use crate::index::graph::Depth;
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

pub fn coverage(ctx: &SearchContext) -> Option<Coverage> {
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
                Verdict::Insufficient(vec![Gap::IncompleteList {
                    of: format!("subclasses of {}", c.root),
                    found: c.found,
                    total: c.total,
                }])
            }
            _ => Verdict::Undecided,
        }
    }
}
