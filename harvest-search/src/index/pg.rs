use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::domain::{Scope, UnitId};
use crate::index::corpus::{Corpus, CorpusSource, Relation, StoredVectors, Unit, UnitKind};

pub const INDEX_SCHEMA: u32 = 1;

const VERSION_SQL: &str = r#"
SELECT v.id, v.parser_version,
       (SELECT count(*) FROM symbols s WHERE s.version_id = v.id) AS symbols,
       (SELECT coalesce(max(s.id), 0) FROM symbols s WHERE s.version_id = v.id) AS max_symbol,
       (SELECT count(*) FROM symbol_edges e JOIN symbols s ON s.id = e.src_id WHERE s.version_id = v.id) AS edges
FROM versions v
JOIN repositories r ON r.id = v.repository_id
WHERE r.name = $repo AND v.tag = $version
"#;

const UNIT_COUNT_SQL: &str =
    "SELECT count(*) AS n, coalesce(max(id), 0) AS m FROM code_units WHERE version_id = $vid";

const CACHE_COUNT_SQL: &str = "SELECT count(*) AS n, coalesce(max(c.id), 0) AS m FROM evidence_cache c JOIN versions v ON v.tag = c.version JOIN repositories r ON r.id = v.repository_id AND r.name = c.repo WHERE v.id = $vid";

const CACHE_SQL: &str = r#"
SELECT c.question, c.evidence, c.symbol_hashes
FROM evidence_cache c
WHERE c.repo = $repo AND c.version = $version
ORDER BY c.id
"#;

const SOURCE_HASHES_SQL: &str =
    "SELECT id, md5(coalesce(source, '')) AS hash FROM symbols WHERE version_id = $vid";

const ENRICHMENT_COUNT_SQL: &str = "SELECT count(*) AS n, coalesce(max(extract(epoch FROM x.updated_at))::bigint, 0) AS m FROM symbol_enrichments x JOIN symbols s ON s.id = x.symbol_id WHERE s.version_id = $vid";

const SYMBOLS_SQL: &str = r#"
SELECT s.id, s.label, s.name, coalesce(s.kind, '') AS kind, coalesce(s.signature, '') AS signature,
       coalesce(s.start_line, 0) AS start_line, coalesce(s.end_line, 0) AS end_line,
       coalesce(s.source, '') AS source, s.impl_type, coalesce(s.bases, '{}') AS bases,
       coalesce(s.traits, '{}') AS traits, coalesce(s.docstring, '') AS docstring,
       coalesce(s.decorators, '{}') AS decorators, f.path, coalesce(f.language, '') AS language
FROM symbols s
JOIN files f ON f.id = s.file_id
WHERE s.version_id = $vid
ORDER BY s.id
"#;

const EDGES_SQL: &str = r#"
SELECT e.src_id, e.dst_id, e.relation
FROM symbol_edges e
JOIN symbols s ON s.id = e.src_id
WHERE s.version_id = $vid
"#;

const UNITS_SQL: &str = r#"
SELECT u.id, u.kind, u.name, u.path, coalesce(u.language, '') AS language,
       u.start_line, u.end_line, u.text, u.symbol_id
FROM code_units u
WHERE u.version_id = $vid AND (u.symbol_id IS NULL OR u.kind = 'chunk')
ORDER BY u.id
"#;

const UNIT_EDGES_SQL: &str = r#"
SELECT e.src_unit_id, e.src_symbol_id, e.dst_symbol_id, e.dst_unit_id, e.relation
FROM unit_edges e
LEFT JOIN code_units u ON u.id = e.src_unit_id
LEFT JOIN symbols s ON s.id = e.src_symbol_id
WHERE coalesce(u.version_id, s.version_id) = $vid
"#;

const ENRICHMENTS_SQL: &str = r#"
SELECT x.symbol_id, x.summary, x.questions, x.tags
FROM symbol_enrichments x
JOIN symbols s ON s.id = x.symbol_id
WHERE s.version_id = $vid
"#;

const EMBEDDINGS_SQL: &str = r#"
SELECT s.id, s.embedding::text AS embedding
FROM symbols s
WHERE s.version_id = $vid AND s.embedding IS NOT NULL AND s.embedding_model = $model
"#;

const HAS_TABLE_SQL: &str = "SELECT to_regclass($name) IS NOT NULL AS present";

const HAS_EMBEDDINGS_SQL: &str = "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'symbols' AND column_name = 'embedding') AS present";

pub struct PgCorpus {
    db: Arc<Db>,
    stored_embedding_model: Option<String>,
}

impl PgCorpus {
    pub fn new(db: Arc<Db>) -> Arc<Self> {
        Arc::new(Self {
            db,
            stored_embedding_model: None,
        })
    }

    pub fn with_stored_embeddings(db: Arc<Db>, model: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            db,
            stored_embedding_model: Some(model.into()),
        })
    }

    async fn version(&self, scope: &Scope) -> Result<Option<Value>> {
        let rows = self
            .db
            .query(
                VERSION_SQL,
                json!({ "repo": scope.repo, "version": scope.version }),
            )
            .await?;
        let Some(mut row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let vid = row["id"].as_i64().unwrap_or(0);
        for (table, sql, field) in [
            ("code_units", UNIT_COUNT_SQL, "units"),
            ("symbol_enrichments", ENRICHMENT_COUNT_SQL, "enrichments"),
            ("evidence_cache", CACHE_COUNT_SQL, "cache"),
        ] {
            let mut fingerprint = String::from("0");
            if self.present(HAS_TABLE_SQL, json!({ "name": table })).await {
                if let Some(counts) = self.db.query(sql, json!({ "vid": vid })).await?.first() {
                    fingerprint = format!(
                        "{}.{}",
                        counts["n"].as_i64().unwrap_or(0),
                        counts["m"].as_i64().unwrap_or(0)
                    );
                }
            }
            row[field] = json!(fingerprint);
        }
        Ok(Some(row))
    }

    async fn present(&self, sql: &str, params: Value) -> bool {
        self.db
            .query(sql, params)
            .await
            .ok()
            .and_then(|rows| rows.first().and_then(|r| r["present"].as_bool()))
            .unwrap_or(false)
    }
}

fn generation(row: &Value) -> String {
    format!(
        "s{INDEX_SCHEMA}-v{}-p{}-n{}-m{}-e{}-u{}-x{}-c{}",
        row["id"].as_i64().unwrap_or(0),
        row["parser_version"].as_i64().unwrap_or(0),
        row["symbols"].as_i64().unwrap_or(0),
        row["max_symbol"].as_i64().unwrap_or(0),
        row["edges"].as_i64().unwrap_or(0),
        row["units"].as_str().unwrap_or("0"),
        row["enrichments"].as_str().unwrap_or("0"),
        row["cache"].as_str().unwrap_or("0"),
    )
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn line(value: &Value) -> u32 {
    value.as_i64().unwrap_or(0).max(0) as u32
}

pub fn symbol_unit(row: &Value) -> Unit {
    let name = text(&row["name"]);
    let parent = row["impl_type"]
        .as_str()
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let kind = if row["label"] == "Class" {
        UnitKind::Class
    } else if parent.is_some() {
        UnitKind::Method
    } else {
        UnitKind::Function
    };
    let qualified_name = match &parent {
        Some(parent) => format!("{parent}.{name}"),
        None => name.clone(),
    };
    let mut bases = strings(&row["bases"]);
    bases.extend(strings(&row["traits"]));
    Unit {
        id: 0,
        db_id: row["id"].as_i64().unwrap_or(0),
        kind: Some(kind),
        file: text(&row["path"]),
        name,
        qualified_name,
        parent,
        signature: text(&row["signature"]),
        docstring: text(&row["docstring"]),
        body: text(&row["source"]),
        start_line: line(&row["start_line"]),
        end_line: line(&row["end_line"]),
        decorators: strings(&row["decorators"]),
        bases,
        language: text(&row["language"]),
        summary: String::new(),
    }
}

pub fn link_members(corpus: &mut Corpus) {
    let classes: HashMap<(String, String), UnitId> = corpus
        .units
        .iter()
        .filter(|u| u.kind() == UnitKind::Class)
        .map(|u| ((u.file.clone(), u.name.clone()), u.id))
        .collect();
    let members: Vec<(UnitId, UnitId)> = corpus
        .units
        .iter()
        .filter_map(|u| {
            let parent = u.parent.as_ref()?;
            let class = classes.get(&(u.file.clone(), parent.clone()))?;
            Some((*class, u.id))
        })
        .collect();
    for (class, member) in members {
        corpus.link(class, member, Relation::Contains);
    }
}

pub fn cached_entry(
    row: &Value,
    corpus: &Corpus,
    by_key: &HashMap<(String, String), UnitId>,
    hashes: &HashMap<i64, String>,
) -> Option<crate::index::corpus::CachedEvidence> {
    let mut units = Vec::new();
    for key in row["evidence"].as_array()? {
        let file = key["file"].as_str().unwrap_or_default().to_string();
        let unit = match key["name"].as_str() {
            Some(name) => by_key.get(&(file, name.to_string())).copied(),
            None => {
                let start = key["start_line"].as_u64().unwrap_or(0) as u32;
                let end = key["end_line"].as_u64().unwrap_or(0) as u32;
                corpus
                    .units
                    .iter()
                    .filter(|u| u.file == file && u.start_line <= end && start <= u.end_line)
                    .min_by_key(|u| u.end_line.saturating_sub(u.start_line))
                    .map(|u| u.id)
            }
        };
        if let Some(unit) = unit {
            if !units.contains(&unit) {
                units.push(unit);
            }
        }
    }
    let stale = row["symbol_hashes"].as_object().is_some_and(|expected| {
        expected.iter().any(|(key, hash)| {
            let unit = key
                .split_once("::")
                .and_then(|(file, name)| by_key.get(&(file.to_string(), name.to_string())));
            match unit {
                Some(unit) => {
                    let db_id = corpus.units[*unit as usize].db_id;
                    hashes.get(&db_id).map(String::as_str) != hash.as_str()
                }
                None => true,
            }
        })
    });
    (!stale && !units.is_empty()).then(|| crate::index::corpus::CachedEvidence {
        question: text(&row["question"]),
        units,
    })
}

fn parse_vector(text: &str) -> Option<Vec<f32>> {
    text.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|v| v.trim().parse::<f32>().ok())
        .collect()
}

#[async_trait]
impl CorpusSource for PgCorpus {
    async fn generation(&self, scope: &Scope) -> Result<Option<String>> {
        Ok(self.version(scope).await?.map(|row| generation(&row)))
    }

    async fn load(&self, scope: &Scope) -> Result<Corpus> {
        let row = self
            .version(scope)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown version {scope:?}"))?;
        let vid = row["id"].as_i64().unwrap_or(0);
        let mut corpus = Corpus::new(scope.clone(), generation(&row));
        let mut by_symbol: HashMap<i64, UnitId> = HashMap::new();
        for symbol in self.db.query(SYMBOLS_SQL, json!({ "vid": vid })).await? {
            let unit = symbol_unit(&symbol);
            let db_id = unit.db_id;
            by_symbol.insert(db_id, corpus.push_unit(unit));
        }
        for edge in self.db.query(EDGES_SQL, json!({ "vid": vid })).await? {
            let (Some(src), Some(dst), Some(relation)) = (
                edge["src_id"].as_i64().and_then(|id| by_symbol.get(&id)),
                edge["dst_id"].as_i64().and_then(|id| by_symbol.get(&id)),
                edge["relation"].as_str().and_then(Relation::parse),
            ) else {
                continue;
            };
            corpus.link(*src, *dst, relation);
        }
        link_members(&mut corpus);

        if self
            .present(HAS_TABLE_SQL, json!({ "name": "symbol_enrichments" }))
            .await
        {
            for row in self
                .db
                .query(ENRICHMENTS_SQL, json!({ "vid": vid }))
                .await?
            {
                if let Some(unit) = row["symbol_id"].as_i64().and_then(|id| by_symbol.get(&id)) {
                    let mut summary = text(&row["summary"]);
                    for extra in strings(&row["questions"])
                        .into_iter()
                        .chain(strings(&row["tags"]))
                    {
                        summary.push('\n');
                        summary.push_str(&extra);
                    }
                    corpus.units[*unit as usize].summary = summary;
                }
            }
        }

        if self
            .present(HAS_TABLE_SQL, json!({ "name": "code_units" }))
            .await
        {
            let mut by_unit: HashMap<i64, UnitId> = HashMap::new();
            for row in self.db.query(UNITS_SQL, json!({ "vid": vid })).await? {
                let Some(kind) = row["kind"].as_str().and_then(UnitKind::parse) else {
                    continue;
                };
                let name = text(&row["name"]);
                let parent_symbol = row["symbol_id"].as_i64();
                let id = corpus.push_unit(Unit {
                    db_id: -row["id"].as_i64().unwrap_or(0),
                    kind: Some(kind),
                    file: text(&row["path"]),
                    qualified_name: name.clone(),
                    name,
                    body: text(&row["text"]),
                    start_line: line(&row["start_line"]),
                    end_line: line(&row["end_line"]),
                    language: text(&row["language"]),
                    ..Unit::default()
                });
                if let Some(owner) = parent_symbol.and_then(|s| by_symbol.get(&s)) {
                    corpus.link(*owner, id, Relation::Contains);
                }
                by_unit.insert(row["id"].as_i64().unwrap_or(0), id);
            }
            if self
                .present(HAS_TABLE_SQL, json!({ "name": "unit_edges" }))
                .await
            {
                for edge in self.db.query(UNIT_EDGES_SQL, json!({ "vid": vid })).await? {
                    let src = edge["src_unit_id"]
                        .as_i64()
                        .and_then(|id| by_unit.get(&id))
                        .or_else(|| {
                            edge["src_symbol_id"]
                                .as_i64()
                                .and_then(|id| by_symbol.get(&id))
                        });
                    let dst = edge["dst_symbol_id"]
                        .as_i64()
                        .and_then(|id| by_symbol.get(&id))
                        .or_else(|| edge["dst_unit_id"].as_i64().and_then(|id| by_unit.get(&id)));
                    let relation = edge["relation"].as_str().and_then(Relation::parse);
                    if let (Some(src), Some(dst), Some(relation)) = (src, dst, relation) {
                        corpus.link(*src, *dst, relation);
                    }
                }
            }
        }

        if self
            .present(HAS_TABLE_SQL, json!({ "name": "evidence_cache" }))
            .await
        {
            let hashes: HashMap<i64, String> = self
                .db
                .query(SOURCE_HASHES_SQL, json!({ "vid": vid }))
                .await?
                .iter()
                .map(|r| (r["id"].as_i64().unwrap_or(0), text(&r["hash"])))
                .collect();
            let by_key: HashMap<(String, String), UnitId> = corpus
                .units
                .iter()
                .map(|u| ((u.file.clone(), u.qualified_name.clone()), u.id))
                .collect();
            let rows = self
                .db
                .query(
                    CACHE_SQL,
                    json!({ "repo": scope.repo, "version": scope.version }),
                )
                .await?;
            for row in rows {
                if let Some(entry) = cached_entry(&row, &corpus, &by_key, &hashes) {
                    corpus.cached.push(entry);
                }
            }
        }

        if let Some(model) = &self.stored_embedding_model {
            if self.present(HAS_EMBEDDINGS_SQL, json!({})).await {
                let mut vectors: Vec<Option<Vec<f32>>> = vec![None; corpus.units.len()];
                for row in self
                    .db
                    .query(EMBEDDINGS_SQL, json!({ "vid": vid, "model": model }))
                    .await?
                {
                    let unit = row["id"].as_i64().and_then(|id| by_symbol.get(&id));
                    let vector = row["embedding"].as_str().and_then(parse_vector);
                    if let (Some(unit), Some(vector)) = (unit, vector) {
                        vectors[*unit as usize] = Some(vector);
                    }
                }
                if vectors.iter().any(Option::is_some) {
                    corpus.vectors = Some(StoredVectors {
                        model: model.clone(),
                        vectors,
                    });
                }
            }
        }
        Ok(corpus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_are_qualified_by_their_type_and_classes_contain_them() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        corpus.push_unit(symbol_unit(&json!({
            "id": 1, "label": "Class", "name": "Stack", "path": "a.rs", "bases": ["Base"], "traits": ["Push"],
        })));
        corpus.push_unit(symbol_unit(&json!({
            "id": 2, "label": "Function", "name": "push", "impl_type": "Stack", "path": "a.rs",
        })));
        corpus.push_unit(symbol_unit(&json!({
            "id": 3, "label": "Function", "name": "free", "impl_type": null, "path": "a.rs",
        })));
        link_members(&mut corpus);
        assert_eq!(corpus.units[0].bases, vec!["Base", "Push"]);
        assert_eq!(corpus.units[1].qualified_name, "Stack.push");
        assert_eq!(corpus.units[1].kind(), UnitKind::Method);
        assert_eq!(corpus.units[2].kind(), UnitKind::Function);
        assert_eq!(corpus.edges.len(), 1);
        assert_eq!(corpus.edges[0].relation, Relation::Contains);
    }

    #[test]
    fn cached_entries_resolve_keys_and_drop_when_a_hash_changed() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        corpus.push_unit(symbol_unit(&json!({"id": 7, "label": "Class", "name": "Driver", "path": "d.py", "start_line": 1, "end_line": 9})));
        let by_key = HashMap::from([(("d.py".to_string(), "Driver".to_string()), 0)]);
        let row = json!({
            "question": "q",
            "evidence": [{"type": "symbol", "file": "d.py", "name": "Driver"}, {"type": "span", "file": "d.py", "start_line": 3, "end_line": 4}],
            "symbol_hashes": {"d.py::Driver": "abc"}
        });
        let fresh = HashMap::from([(7, "abc".to_string())]);
        assert_eq!(
            cached_entry(&row, &corpus, &by_key, &fresh).unwrap().units,
            vec![0]
        );
        let changed = HashMap::from([(7, "xyz".to_string())]);
        assert!(cached_entry(&row, &corpus, &by_key, &changed).is_none());
    }

    #[test]
    fn pgvector_text_parses() {
        assert_eq!(parse_vector("[1,0.5,-2]"), Some(vec![1.0, 0.5, -2.0]));
        assert_eq!(parse_vector("[a]"), None);
    }
}
