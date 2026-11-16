use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::Gap;
use crate::gate::{SufficiencyCheck, Verdict};
use crate::question::QuestionKind;

pub struct FamilyProperty;

#[async_trait]
impl SufficiencyCheck for FamilyProperty {
    fn name(&self) -> &'static str {
        "family_property"
    }

    async fn check(&self, ctx: &SearchContext) -> Verdict {
        let family_question =
            ctx.question.kind == QuestionKind::Hierarchy && ctx.question.wants_list;
        if !family_question || !ctx.question.asks_property {
            return Verdict::Undecided;
        }
        let resolved = ctx.evidence.iter().any(|e| {
            e.label.starts_with("capability ") || e.label.starts_with("implementations of ")
        });
        if resolved {
            Verdict::Undecided
        } else {
            Verdict::Insufficient(vec![Gap::Unanswered {
                hint: "the evidence lists the classes but not which of them have the property asked about".into(),
            }])
        }
    }
}
