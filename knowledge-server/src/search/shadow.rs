use std::collections::HashMap;
use std::sync::Arc;

use harvest_search::{Evidence, EvidenceKey};

use crate::agent::Source as Citation;
use crate::search::record::TraceContext;
use crate::search::SearchService;

pub fn dominant_scope(citations: &[Citation]) -> Option<(String, String)> {
    let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
    for c in citations {
        *counts
            .entry((c.repo.as_str(), c.version.as_str()))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|((repo, version), _)| (repo.to_string(), version.to_string()))
}

fn covers(evidence: &Evidence, citation: &Citation) -> bool {
    let end = citation.end_line.unwrap_or(citation.line);
    let spans = evidence.keys().filter_map(|key| match key {
        EvidenceKey::Span {
            file,
            start_line,
            end_line,
        } => Some((file.as_str(), *start_line, *end_line)),
        EvidenceKey::Symbol { file, .. } if !file.is_empty() => {
            Some((file.as_str(), evidence.start_line, evidence.end_line))
        }
        EvidenceKey::Symbol { .. } => None,
    });
    for (file, start, stop) in spans {
        if file == citation.file && (citation.line == 0 || (start <= end && citation.line <= stop))
        {
            return true;
        }
    }
    false
}

pub fn recall(evidence: &[Evidence], citations: &[Citation]) -> Option<f64> {
    if citations.is_empty() {
        return None;
    }
    let found = citations
        .iter()
        .filter(|c| evidence.iter().any(|e| covers(e, c)))
        .count();
    Some(found as f64 / citations.len() as f64)
}

pub fn spawn(
    service: Arc<SearchService>,
    question: String,
    citations: Vec<Citation>,
    turn_id: String,
    conversation_id: String,
) {
    let Some((repo, version)) = dominant_scope(&citations) else {
        return;
    };
    tokio::spawn(async move {
        let query = service.query(&question, &repo, &version);
        let scoped: Vec<Citation> = citations
            .into_iter()
            .filter(|c| c.repo == repo && c.version == version)
            .collect();
        let result = match service.engine.search(&query, service.deadline()).await {
            Ok(result) => result,
            Err(e) => {
                tracing::debug!(error = %e, "shadow search failed");
                return;
            }
        };
        let context = TraceContext {
            engine: "shadow",
            turn_id: Some(turn_id),
            conversation_id: Some(conversation_id),
            shadow_recall: recall(&result.evidence, &scoped),
        };
        service.recorder.record(&query, &result, context);
    });
}

pub fn tap(
    mut events: tokio::sync::mpsc::Receiver<crate::agent::AgentEvent>,
    service: Arc<SearchService>,
    question: String,
    turn_id: String,
    conversation_id: String,
) -> tokio::sync::mpsc::Receiver<crate::agent::AgentEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let crate::agent::AgentEvent::Done { sources, .. } = &event {
                spawn(
                    Arc::clone(&service),
                    question.clone(),
                    sources.clone(),
                    turn_id.clone(),
                    conversation_id.clone(),
                );
            }
            if tx.send(event).await.is_err() {
                return;
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::Source;

    fn citation(repo: &str, file: &str, line: u32) -> Citation {
        Citation {
            repo: repo.into(),
            version: "v".into(),
            file: file.into(),
            line,
            end_line: Some(line + 2),
        }
    }

    fn evidence(file: &str, lines: (u32, u32)) -> Evidence {
        Evidence {
            key: EvidenceKey::Symbol {
                file: file.into(),
                name: "f".into(),
            },
            unit: Some(1),
            label: String::new(),
            start_line: lines.0,
            end_line: lines.1,
            text: String::new(),
            score: 1.0,
            found_by: Source::Lexical,
            covers: Vec::new(),
        }
    }

    #[test]
    fn the_most_cited_scope_wins() {
        let citations = vec![
            citation("a", "x", 1),
            citation("b", "y", 1),
            citation("b", "z", 1),
        ];
        assert_eq!(dominant_scope(&citations), Some(("b".into(), "v".into())));
        assert_eq!(dominant_scope(&[]), None);
    }

    #[test]
    fn recall_counts_citations_overlapped_by_evidence() {
        let citations = vec![citation("r", "a.py", 10), citation("r", "b.py", 50)];
        let found = vec![evidence("a.py", (5, 11)), evidence("b.py", (1, 20))];
        assert_eq!(recall(&found, &citations), Some(0.5));
        assert_eq!(recall(&found, &[]), None);
    }
}
