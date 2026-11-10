use std::collections::HashSet;

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::Gap;
use crate::gate::{SufficiencyCheck, Verdict};
use crate::index::corpus::Relation;
use crate::index::graph::Direction;

const FOLLOWED: [Relation; 7] = [
    Relation::Calls,
    Relation::Inherits,
    Relation::Implements,
    Relation::ReadsConfig,
    Relation::Instantiates,
    Relation::Overrides,
    Relation::Uses,
];
const MAX_GAPS: usize = 8;
const CORE_EVIDENCE: usize = 5;

pub fn dangling(ctx: &SearchContext) -> Vec<Gap> {
    let index = &ctx.index;
    let mut gaps: Vec<Gap> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let core: Vec<_> = ctx
        .evidence
        .iter()
        .filter_map(|e| e.unit)
        .take(CORE_EVIDENCE)
        .collect();
    for unit_id in core {
        let unit = index.unit(unit_id);
        for target in index.graph.neighbors(unit_id, Direction::Out, &FOLLOWED) {
            let referenced = index.unit(target);
            if ctx.has_unit(target) || !seen.insert(referenced.qualified_name.clone()) {
                continue;
            }
            if ctx.question.relates_to(&referenced.qualified_name) {
                gaps.push(Gap::DanglingReference {
                    name: referenced.qualified_name.clone(),
                    referenced_by: unit.qualified_name.clone(),
                });
            }
            if gaps.len() >= MAX_GAPS {
                return gaps;
            }
        }
    }
    gaps
}

pub struct DanglingReferences;

#[async_trait]
impl SufficiencyCheck for DanglingReferences {
    fn name(&self) -> &'static str {
        "dangling_references"
    }

    async fn check(&self, ctx: &SearchContext) -> Verdict {
        let gaps = dangling(ctx);
        if gaps.is_empty() {
            Verdict::Undecided
        } else {
            Verdict::Insufficient(gaps)
        }
    }
}
