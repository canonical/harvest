use std::collections::HashMap;

use anyhow::Result;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::gold::{GoldEvidence, GoldQuery};

const SYMBOLS_SQL: &str = r#"
SELECT id, file, name, impl_type, label, coalesce(start_line, 0) AS start_line,
       coalesce(end_line, 0) AS end_line, coalesce(docstring, '') AS docstring
FROM code_symbols
WHERE repo = $repo AND version = $version
  AND file !~* '(^|/)(tests?|testing)(/|_)|_test\.|test_'
ORDER BY md5(id::text)
LIMIT $limit
"#;

const FAMILIES_SQL: &str = r#"
SELECT e.dst_id AS id
FROM code_edges e
WHERE e.repo = $repo AND e.version = $version AND e.relation IN ('INHERITS', 'IMPLEMENTS')
  AND e.src_file !~* '(^|/)(tests?|testing)(/|_)|_test\.|test_'
GROUP BY e.dst_id
HAVING count(*) >= 3
ORDER BY md5(e.dst_id::text)
LIMIT $limit
"#;

const DESCENDANTS_SQL: &str = r#"
WITH RECURSIVE family(id) AS (
    SELECT src_id FROM symbol_edges WHERE dst_id = $root AND relation IN ('INHERITS', 'IMPLEMENTS')
    UNION
    SELECT e.src_id FROM symbol_edges e JOIN family f ON e.dst_id = f.id
    WHERE e.relation IN ('INHERITS', 'IMPLEMENTS')
)
SELECT s.id, s.file, s.name, s.impl_type, s.label, coalesce(s.start_line, 0) AS start_line,
       coalesce(s.end_line, 0) AS end_line, '' AS docstring
FROM family JOIN code_symbols s ON s.id = family.id
WHERE s.file !~* '(^|/)(tests?|testing)(/|_)|_test\.|test_'
"#;

const CALL_TARGETS_SQL: &str = r#"
SELECT e.dst_id AS id, count(*) AS n
FROM code_edges e
WHERE e.repo = $repo AND e.version = $version AND e.relation = 'CALLS'
GROUP BY e.dst_id
HAVING count(*) BETWEEN 2 AND 15
ORDER BY md5(e.dst_id::text)
LIMIT $limit
"#;

const NEIGHBORS_SQL: &str = r#"
SELECT s.id, s.file, s.name, s.impl_type, s.label, coalesce(s.start_line, 0) AS start_line,
       coalesce(s.end_line, 0) AS end_line, '' AS docstring
FROM symbol_edges e
JOIN code_symbols s ON s.id = CASE WHEN $direction = 'in' THEN e.src_id ELSE e.dst_id END
WHERE e.relation = 'CALLS'
  AND CASE WHEN $direction = 'in' THEN e.dst_id ELSE e.src_id END = $id
"#;

const NAME_COUNTS_SQL: &str = r#"
SELECT coalesce(impl_type || '.', '') || name AS qualified, count(*) AS n
FROM code_symbols
WHERE repo = $repo AND version = $version
GROUP BY 1
"#;

const SYMBOL_SQL: &str = r#"
SELECT id, file, name, impl_type, label, coalesce(start_line, 0) AS start_line,
       coalesce(end_line, 0) AS end_line, coalesce(docstring, '') AS docstring
FROM code_symbols WHERE id = $id
"#;

#[derive(Clone, Debug)]
pub struct Symbol {
    pub id: i64,
    pub file: String,
    pub name: String,
    pub qualified: String,
    pub is_class: bool,
    pub lines: (u32, u32),
    pub docstring: String,
}

fn symbol(row: &Value) -> Symbol {
    let name = row["name"].as_str().unwrap_or_default().to_string();
    let qualified = match row["impl_type"].as_str().filter(|p| !p.is_empty()) {
        Some(parent) => format!("{parent}.{name}"),
        None => name.clone(),
    };
    Symbol {
        id: row["id"].as_i64().unwrap_or(0),
        file: row["file"].as_str().unwrap_or_default().to_string(),
        name,
        qualified,
        is_class: row["label"] == "Class",
        lines: (
            row["start_line"].as_i64().unwrap_or(0) as u32,
            row["end_line"].as_i64().unwrap_or(0) as u32,
        ),
        docstring: row["docstring"].as_str().unwrap_or_default().to_string(),
    }
}

fn evidence(s: &Symbol, essential: bool) -> GoldEvidence {
    GoldEvidence::symbol(&s.file, &s.qualified, s.lines, essential)
}

pub fn first_sentence(docstring: &str) -> Option<String> {
    let text = docstring
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '/' || c == '*')
        .trim();
    let sentence = text
        .split(['\n', '.'])
        .map(str::trim)
        .find(|s| !s.is_empty())?;
    let words = sentence.split_whitespace().count();
    (4..=30).contains(&words).then(|| sentence.to_string())
}

pub struct Synthesizer<'a> {
    db: &'a Db,
    repo: String,
    version: String,
}

impl<'a> Synthesizer<'a> {
    pub fn new(db: &'a Db, repo: &str, version: &str) -> Self {
        Self {
            db,
            repo: repo.into(),
            version: version.into(),
        }
    }

    fn query(
        &self,
        kind: &str,
        key: &str,
        question: String,
        evidence: Vec<GoldEvidence>,
    ) -> GoldQuery {
        GoldQuery {
            id: format!("synth-{kind}-{}-{}-{key}", self.repo, self.version),
            question,
            repo: self.repo.clone(),
            version: self.version.clone(),
            kind: kind.into(),
            source: "synthetic".into(),
            evidence,
            reference_answer: None,
        }
    }

    async fn rows(&self, sql: &str, extra: Value) -> Result<Vec<Value>> {
        let mut params = json!({ "repo": self.repo, "version": self.version });
        if let (Some(target), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                target.insert(k.clone(), v.clone());
            }
        }
        self.db.query(sql, params).await
    }

    async fn name_counts(&self) -> Result<HashMap<String, i64>> {
        Ok(self
            .rows(NAME_COUNTS_SQL, json!({}))
            .await?
            .iter()
            .map(|r| {
                (
                    r["qualified"].as_str().unwrap_or_default().to_string(),
                    r["n"].as_i64().unwrap_or(0),
                )
            })
            .collect())
    }

    async fn by_id(&self, id: i64) -> Result<Option<Symbol>> {
        Ok(self
            .rows(SYMBOL_SQL, json!({ "id": id }))
            .await?
            .first()
            .map(symbol))
    }

    pub async fn generate(&self, per_kind: usize) -> Result<Vec<GoldQuery>> {
        let mut queries = Vec::new();
        let sample = self
            .rows(SYMBOLS_SQL, json!({ "limit": (per_kind * 12) as i64 }))
            .await?;
        let counts = self.name_counts().await?;
        let unambiguous = |s: &Symbol| counts.get(&s.qualified).copied().unwrap_or(0) == 1;
        let symbols: Vec<Symbol> = sample.iter().map(symbol).collect();
        for s in symbols.iter().filter(|s| unambiguous(s)).take(per_kind) {
            let what = if s.is_class { "class" } else { "function" };
            queries.push(self.query(
                "lookup",
                &s.id.to_string(),
                format!(
                    "Where is the {what} {} defined and what does it do?",
                    s.qualified
                ),
                vec![evidence(s, true)],
            ));
        }
        for s in symbols
            .iter()
            .filter(|s| first_sentence(&s.docstring).is_some())
            .take(per_kind)
        {
            let sentence = first_sentence(&s.docstring).unwrap_or_default();
            queries.push(self.query(
                "describe",
                &s.id.to_string(),
                format!("Find the code that does this: {sentence}"),
                vec![evidence(s, true)],
            ));
        }
        for row in self
            .rows(FAMILIES_SQL, json!({ "limit": per_kind as i64 }))
            .await?
        {
            let Some(root) = self.by_id(row["id"].as_i64().unwrap_or(0)).await? else {
                continue;
            };
            let family: Vec<Symbol> = self
                .db
                .query(DESCENDANTS_SQL, json!({ "root": root.id }))
                .await?
                .iter()
                .map(symbol)
                .collect();
            if family.len() < 3 {
                continue;
            }
            queries.push(self.query(
                "list",
                &root.id.to_string(),
                format!("Which classes extend {}?", root.qualified),
                family.iter().map(|s| evidence(s, true)).collect(),
            ));
        }
        for row in self
            .rows(CALL_TARGETS_SQL, json!({ "limit": (per_kind * 6) as i64 }))
            .await?
        {
            if queries.iter().filter(|q| q.kind == "callers").count() >= per_kind {
                break;
            }
            let Some(target) = self.by_id(row["id"].as_i64().unwrap_or(0)).await? else {
                continue;
            };
            if !unambiguous(&target) {
                continue;
            }
            let callers: Vec<Symbol> = self
                .db
                .query(NEIGHBORS_SQL, json!({ "direction": "in", "id": target.id }))
                .await?
                .iter()
                .map(symbol)
                .collect();
            let mut gold: Vec<GoldEvidence> = callers.iter().map(|s| evidence(s, true)).collect();
            gold.push(evidence(&target, false));
            queries.push(self.query(
                "callers",
                &target.id.to_string(),
                format!("What calls {}?", target.qualified),
                gold,
            ));
            let callees: Vec<Symbol> = self
                .db
                .query(
                    NEIGHBORS_SQL,
                    json!({ "direction": "out", "id": target.id }),
                )
                .await?
                .iter()
                .map(symbol)
                .collect();
            if !callees.is_empty() {
                let mut gold = vec![evidence(&target, true)];
                gold.extend(callees.iter().take(10).map(|s| evidence(s, false)));
                queries.push(self.query(
                    "flow",
                    &target.id.to_string(),
                    format!("What happens when {} is called?", target.qualified),
                    gold,
                ));
            }
        }
        let mut seen: HashMap<String, ()> = HashMap::new();
        queries.retain(|q| seen.insert(q.id.clone(), ()).is_none());
        Ok(queries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sentences_need_a_reasonable_length() {
        assert_eq!(
            first_sentence("\"\"\"Return the attachment for a volume.\n\nMore.\"\"\"").as_deref(),
            Some("Return the attachment for a volume")
        );
        assert_eq!(first_sentence("Init."), None);
        assert_eq!(first_sentence(""), None);
    }
}
