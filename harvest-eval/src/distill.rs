use std::collections::BTreeMap;

use anyhow::Result;
use harvest_db::Db;
use harvest_search::{EvidenceKey, SearchResult};
use serde_json::{json, Value};

use crate::gold::{GoldEvidence, GoldQuery};

const HASH_SQL: &str = r#"
SELECT md5(coalesce(source, '')) AS hash
FROM code_symbols
WHERE repo = $repo AND version = $version AND file = $file
  AND (coalesce(impl_type || '.', '') || name) = $name
LIMIT 1
"#;

const INSERT_SQL: &str = r#"
INSERT INTO evidence_cache (repo, version, question, evidence, symbol_hashes, source)
VALUES ($repo, $version, $question, $evidence, $hashes, $source)
ON CONFLICT (repo, version, question) DO UPDATE
   SET evidence = EXCLUDED.evidence, symbol_hashes = EXCLUDED.symbol_hashes,
       source = EXCLUDED.source, created_at = now()
"#;

pub fn gold_from_result(
    query: &GoldQuery,
    result: &SearchResult,
    engine: &str,
) -> Option<GoldQuery> {
    let mut evidence: Vec<GoldEvidence> = Vec::new();
    for e in &result.evidence {
        for key in e.keys() {
            let gold = match key {
                EvidenceKey::Symbol { file, name } if !file.is_empty() => {
                    let lines = if key == &e.key && e.end_line > 0 {
                        Some((e.start_line, e.end_line))
                    } else {
                        None
                    };
                    GoldEvidence {
                        file: file.clone(),
                        name: Some(name.clone()),
                        start_line: lines.map(|l| l.0),
                        end_line: lines.map(|l| l.1),
                        essential: true,
                    }
                }
                EvidenceKey::Span {
                    file,
                    start_line,
                    end_line,
                } => GoldEvidence::span(file, (*start_line, *end_line), true),
                EvidenceKey::Symbol { .. } => continue,
            };
            if !evidence.iter().any(|g| {
                g.file == gold.file && g.name == gold.name && g.start_line == gold.start_line
            }) {
                evidence.push(gold);
            }
        }
    }
    (!evidence.is_empty()).then(|| GoldQuery {
        id: format!("offline-{engine}-{}", query.id),
        question: query.question.clone(),
        repo: query.repo.clone(),
        version: query.version.clone(),
        kind: query.kind.clone(),
        source: format!("offline:{engine}"),
        evidence,
        reference_answer: result.answer.clone(),
    })
}

pub fn cache_keys(gold: &GoldQuery) -> Vec<Value> {
    gold.evidence
        .iter()
        .map(|g| match (&g.name, g.start_line) {
            (Some(name), _) => json!({ "type": "symbol", "file": g.file, "name": name }),
            (None, Some(start)) => json!({
                "type": "span", "file": g.file, "start_line": start,
                "end_line": g.end_line.unwrap_or(start)
            }),
            (None, None) => {
                json!({ "type": "span", "file": g.file, "start_line": 1, "end_line": u32::MAX })
            }
        })
        .collect()
}

pub async fn store_cache(db: &Db, gold: &GoldQuery) -> Result<()> {
    let mut hashes: BTreeMap<String, String> = BTreeMap::new();
    for g in &gold.evidence {
        let Some(name) = &g.name else {
            continue;
        };
        let row = db
            .query(
                HASH_SQL,
                json!({ "repo": gold.repo, "version": gold.version, "file": g.file, "name": name }),
            )
            .await?;
        if let Some(hash) = row.first().and_then(|r| r["hash"].as_str()) {
            hashes.insert(format!("{}::{name}", g.file), hash.to_string());
        }
    }
    db.execute(
        INSERT_SQL,
        json!({
            "repo": gold.repo, "version": gold.version, "question": gold.question,
            "evidence": cache_keys(gold), "hashes": hashes, "source": gold.source,
        }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::{Evidence, Source, TierId};

    #[test]
    fn engine_evidence_becomes_offline_gold_and_cache_keys() {
        let query = GoldQuery {
            id: "q1".into(),
            question: "Which drivers support multiattach?".into(),
            repo: "r".into(),
            version: "v".into(),
            kind: "list".into(),
            source: "synthetic".into(),
            evidence: vec![],
            reference_answer: None,
        };
        let result = SearchResult {
            evidence: vec![Evidence {
                key: EvidenceKey::Symbol {
                    file: String::new(),
                    name: "capability x".into(),
                },
                unit: None,
                label: "capability x".into(),
                start_line: 0,
                end_line: 0,
                text: String::new(),
                score: 1.0,
                found_by: Source::Plan,
                covers: vec![
                    EvidenceKey::Symbol {
                        file: "lvm.py".into(),
                        name: "LVM".into(),
                    },
                    EvidenceKey::Span {
                        file: "nfs.py".into(),
                        start_line: 3,
                        end_line: 9,
                    },
                ],
            }],
            gaps: vec![],
            tier_reached: TierId::Loop,
            degraded: None,
            generation: None,
            trace: Default::default(),
            answer: Some("LVM does.".into()),
        };
        let gold = gold_from_result(&query, &result, "legacy").unwrap();
        assert_eq!(gold.id, "offline-legacy-q1");
        assert_eq!(gold.evidence.len(), 2);
        assert_eq!(gold.reference_answer.as_deref(), Some("LVM does."));
        let keys = cache_keys(&gold);
        assert_eq!(keys[0]["name"], "LVM");
        assert_eq!(keys[1]["start_line"], 3);
    }
}
