use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::Gap;
use crate::gate::{SufficiencyCheck, Verdict};

pub fn missing(ctx: &SearchContext) -> Vec<Gap> {
    ctx.named
        .iter()
        .filter(|unit| !ctx.has_unit(**unit))
        .map(|unit| Gap::Unanswered {
            hint: format!(
                "{} is named in the question but is not in the evidence",
                ctx.index.unit(*unit).qualified_name
            ),
        })
        .collect()
}

pub struct NamedSymbols;

#[async_trait]
impl SufficiencyCheck for NamedSymbols {
    fn name(&self) -> &'static str {
        "named_symbols"
    }

    async fn check(&self, ctx: &SearchContext) -> Verdict {
        let gaps = missing(ctx);
        if gaps.is_empty() {
            Verdict::Undecided
        } else {
            Verdict::Insufficient(gaps)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::domain::{Evidence, Query, Scope, Source};
    use crate::index::corpus::{Corpus, Unit, UnitKind};
    use crate::index::vectors::HashingEmbedder;
    use crate::index::IndexSnapshot;
    use crate::retrieve::pack::evidence_for;
    use crate::Deadline;

    #[tokio::test]
    async fn rust_style_paths_resolve_and_missing_ones_are_gaps() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        let commit = corpus.push_unit(Unit {
            name: "commit".into(),
            qualified_name: "Fence.commit".into(),
            parent: Some("Fence".into()),
            kind: Some(UnitKind::Method),
            file: "src/jobs/fence.rs".into(),
            start_line: 10,
            end_line: 20,
            ..Unit::default()
        });
        corpus.push_unit(Unit {
            name: "commit".into(),
            qualified_name: "Tx.commit".into(),
            parent: Some("Tx".into()),
            kind: Some(UnitKind::Method),
            file: "src/db.rs".into(),
            start_line: 1,
            end_line: 5,
            ..Unit::default()
        });
        let index = Arc::new(
            IndexSnapshot::build(corpus, &HashingEmbedder::default())
                .await
                .unwrap(),
        );
        assert_eq!(index.find_by_name("Fence::commit"), &[commit]);
        assert_eq!(index.find_by_name("crate::jobs::Fence::commit"), &[commit]);
        assert_eq!(index.find_by_name("fence->commit()"), &[commit]);
        assert_eq!(index.find_by_name("commit").len(), 2);

        let query = Query::new(
            "How does `Fence::commit` stop stale writes?",
            Scope::new("r", "v"),
        );
        let mut ctx = SearchContext::new(
            &query,
            Arc::clone(&index),
            Deadline::after(std::time::Duration::from_secs(5)),
        );
        assert_eq!(ctx.named, vec![commit]);
        assert!(matches!(
            NamedSymbols.check(&ctx).await,
            Verdict::Insufficient(_)
        ));
        let unit = index.unit(commit).clone();
        ctx.evidence
            .push(evidence_for(&unit, String::new(), 1.0, Source::Exact));
        let _: &Evidence = &ctx.evidence[0];
        assert_eq!(NamedSymbols.check(&ctx).await, Verdict::Undecided);
    }

    #[tokio::test]
    async fn plain_words_that_name_a_rare_symbol_count_as_named() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        let multadd = corpus.push_unit(Unit {
            name: "multadd".into(),
            qualified_name: "multadd".into(),
            kind: Some(UnitKind::Function),
            file: "src/jv_dtoa.c".into(),
            ..Unit::default()
        });
        for file in ["a.c", "b.c", "c.c"] {
            corpus.push_unit(Unit {
                name: "parse".into(),
                qualified_name: "parse".into(),
                kind: Some(UnitKind::Function),
                file: file.into(),
                ..Unit::default()
            });
        }
        let index = Arc::new(
            IndexSnapshot::build(corpus, &HashingEmbedder::default())
                .await
                .unwrap(),
        );
        let query = Query::new(
            "What happens when multadd is called during parse?",
            Scope::new("r", "v"),
        );
        let ctx = SearchContext::new(
            &query,
            index,
            Deadline::after(std::time::Duration::from_secs(5)),
        );
        assert_eq!(ctx.named, vec![multadd], "common names stay unpinned");
    }
}
