pub mod plan;
pub mod rounds;
pub mod steps;

use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::{SearchError, TierId};
use crate::retrieve::pack::{
    code_budget, documentation_budget, pack_documentation, pack_shaped, Shape,
};
use crate::retrieve::rerank::Reranker;
use crate::retrieve::signposts::{follow, merge, split_by_trust, Split};

#[async_trait]
pub trait Tier: Send + Sync {
    fn id(&self) -> TierId;
    fn expected_cost(&self) -> Duration;
    async fn run(&self, ctx: &mut SearchContext) -> Result<(), SearchError>;
}

pub async fn rerank_and_pack(
    ctx: &mut SearchContext,
    reranker: &dyn Reranker,
) -> Result<(), SearchError> {
    let started = Instant::now();
    let Split {
        code,
        documentation,
    } = split_by_trust(&ctx.index, std::mem::take(&mut ctx.candidates));
    merge(&mut ctx.signposts, documentation);
    let code = follow(&ctx.index, code, &ctx.signposts);
    ctx.candidates = reranker.rerank(&ctx.question, &ctx.index, code).await?;
    ctx.trace
        .record(format!("{}.rerank", ctx.tier.as_str()), started.elapsed());
    let started = Instant::now();
    let has_documentation = !ctx.signposts.is_empty();
    ctx.evidence = pack_shaped(
        &ctx.index,
        &ctx.candidates,
        ctx.fixed.clone(),
        code_budget(ctx.query.budget, has_documentation),
        Shape::of(&ctx.question),
    );
    ctx.documentation = pack_documentation(
        &ctx.index,
        &ctx.signposts,
        documentation_budget(ctx.query.budget),
    );
    ctx.trace
        .record(format!("{}.pack", ctx.tier.as_str()), started.elapsed());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::domain::{Query, Scope, TokenBudget};
    use crate::index::corpus::{Corpus, Unit, UnitKind};
    use crate::index::vectors::HashingEmbedder;
    use crate::index::IndexSnapshot;
    use crate::retrieve::pack::DOCUMENTATION_SHARE;
    use crate::retrieve::rerank::LinearReranker;
    use crate::retrieve::Candidate;
    use crate::Deadline;

    fn unit(name: &str, kind: UnitKind, file: &str, body: &str) -> Unit {
        Unit {
            name: name.into(),
            qualified_name: name.into(),
            kind: Some(kind),
            file: file.into(),
            body: body.into(),
            start_line: 1,
            end_line: 20,
            ..Unit::default()
        }
    }

    async fn context(candidates: &[u32]) -> SearchContext {
        let mut corpus = Corpus::new(Scope::new("amafu", "main"), "g");
        corpus.push_unit(unit(
            "Install with MAAS (planned)",
            UnitKind::Doc,
            "README.md",
            &format!(
                "Run `amafu cluster create`; the installer uses `MaasClient`.\n{}",
                "More words. ".repeat(400)
            ),
        ));
        corpus.push_unit(unit(
            "MaasClient",
            UnitKind::Class,
            "crates/installer/src/maas.rs",
            "pub struct MaasClient {}",
        ));
        corpus.push_unit(unit(
            "run",
            UnitKind::Function,
            "crates/command-line/src/install.rs",
            "pub async fn run() {}",
        ));
        let index = Arc::new(
            IndexSnapshot::build(corpus, &HashingEmbedder::default())
                .await
                .unwrap(),
        );
        let mut query = Query::new(
            "How does Amafu get installed through MAAS?",
            Scope::new("amafu", "main"),
        );
        query.budget = TokenBudget(4_000);
        let mut ctx = SearchContext::new(&query, index, Deadline::after(Duration::from_secs(5)));
        ctx.candidates = candidates
            .iter()
            .map(|u| Candidate {
                unit: *u,
                rrf: 0.05 - *u as f32 * 0.01,
                score: 1.0,
                ..Candidate::default()
            })
            .collect();
        ctx
    }

    #[tokio::test]
    async fn documentation_never_becomes_evidence_but_leads_to_the_code() {
        let mut ctx = context(&[0, 2]).await;
        rerank_and_pack(&mut ctx, &LinearReranker::default())
            .await
            .unwrap();
        let evidence: Vec<&str> = ctx.evidence.iter().map(|e| e.label.as_str()).collect();
        assert!(
            evidence.iter().all(|l| !l.starts_with("doc ")),
            "{evidence:?}"
        );
        assert!(
            evidence.iter().any(|l| l.contains("MaasClient")),
            "the documentation led to MaasClient: {evidence:?}"
        );
        assert_eq!(ctx.documentation.len(), 1);
        let doc = &ctx.documentation[0];
        assert!(
            doc.text
                .contains("points to code: MaasClient (crates/installer/src/maas.rs:1)"),
            "{}",
            doc.text
        );
        let doc_chars: usize = ctx.documentation.iter().map(|e| e.text.len()).sum();
        assert!(doc_chars as f32 <= 4_000.0 * 4.0 * DOCUMENTATION_SHARE + 200.0);
    }

    #[tokio::test]
    async fn code_alone_gets_the_whole_budget_and_no_documentation_section() {
        let mut ctx = context(&[1, 2]).await;
        rerank_and_pack(&mut ctx, &LinearReranker::default())
            .await
            .unwrap();
        assert!(ctx.documentation.is_empty());
        assert_eq!(ctx.evidence.len(), 2);
    }

    #[tokio::test]
    async fn documentation_found_in_an_earlier_round_is_kept() {
        let mut ctx = context(&[0, 2]).await;
        rerank_and_pack(&mut ctx, &LinearReranker::default())
            .await
            .unwrap();
        ctx.candidates = vec![Candidate {
            unit: 2,
            score: 1.0,
            ..Candidate::default()
        }];
        rerank_and_pack(&mut ctx, &LinearReranker::default())
            .await
            .unwrap();
        assert_eq!(ctx.documentation.len(), 1);
        assert!(ctx.evidence.iter().any(|e| e.label.contains("MaasClient")));
    }
}
