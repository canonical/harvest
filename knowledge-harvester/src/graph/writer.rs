use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, Result};
use harvest_db::{Db, Tx};
use serde_json::{json, Value};

use super::model::ParsedFile;
use super::resolve::{
    CallRecord, Label, RefKind, ReferenceRecord, ResolvedEdge, Resolver, SymbolRecord,
};
use crate::git::History;
use crate::parser::PARSER_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionStatus {
    NotIngested,
    Stale { parser_version: i32 },
    Current,
}

const EDGE_BATCH: usize = 5_000;

pub struct GraphWriter {
    db: Db,
}

impl GraphWriter {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn reset_ingested(&self) -> Result<()> {
        self.db
            .execute("UPDATE versions SET ingested = false", json!({}))
            .await?;
        Ok(())
    }

    pub async fn is_ingested(&self, repo: &str, tag: &str) -> Result<bool> {
        Ok(self.ingestion_status(repo, tag).await? == IngestionStatus::Current)
    }

    pub async fn ingestion_status(&self, repo: &str, tag: &str) -> Result<IngestionStatus> {
        let rows = self.db.query(
            "SELECT parser_version FROM code_versions WHERE repo = $repo AND tag = $tag AND ingested LIMIT 1",
            json!({ "repo": repo, "tag": tag }),
        ).await?;
        Ok(
            match rows.first().and_then(|r| r["parser_version"].as_i64()) {
                None => IngestionStatus::NotIngested,
                Some(v) if v < i64::from(PARSER_VERSION) => IngestionStatus::Stale {
                    parser_version: v as i32,
                },
                Some(_) => IngestionStatus::Current,
            },
        )
    }

    pub async fn ingested_versions(&self, repo: &str) -> Result<Vec<String>> {
        let rows = self
            .db
            .query(
                "SELECT tag FROM code_versions WHERE repo = $repo AND ingested ORDER BY timestamp",
                json!({ "repo": repo }),
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| r["tag"].as_str().map(String::from))
            .collect())
    }

    pub async fn upsert_repository(&self, name: &str, url: &str) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO repositories (name, url) VALUES ($name, $url)
             ON CONFLICT (name) DO UPDATE SET url = EXCLUDED.url",
                json!({ "name": name, "url": url }),
            )
            .await?;
        Ok(())
    }

    pub async fn upsert_version(
        &self,
        repo: &str,
        tag: &str,
        timestamp: i64,
        ingested: bool,
    ) -> Result<()> {
        self.db
            .execute(
                "WITH r AS (
                 INSERT INTO repositories (name) VALUES ($repo)
                 ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
                 RETURNING id
             )
             INSERT INTO versions (repository_id, tag, timestamp, ingested)
             SELECT id, $tag, $timestamp::bigint, $ingested::boolean FROM r
             ON CONFLICT (repository_id, tag)
                 DO UPDATE SET timestamp = EXCLUDED.timestamp,
                               ingested  = versions.ingested OR EXCLUDED.ingested",
                json!({ "repo": repo, "tag": tag, "timestamp": timestamp, "ingested": ingested }),
            )
            .await?;
        Ok(())
    }

    pub async fn write_version(&self, repo: &str, tag: &str, files: &[ParsedFile]) -> Result<()> {
        self.write_version_with_history(repo, tag, files, &History::default())
            .await
    }

    pub async fn write_version_with_history(
        &self,
        repo: &str,
        tag: &str,
        files: &[ParsedFile],
        history: &History,
    ) -> Result<()> {
        let tx = self.db.begin().await?;
        let version_id = ensure_version(&tx, repo, tag).await?;
        tx.execute(
            "DELETE FROM files WHERE version_id = $vid",
            json!({ "vid": version_id }),
        )
        .await?;
        for file in files {
            write_file(&tx, version_id, file, history).await?;
        }
        write_edges(&tx, version_id, files, history).await?;
        tx.execute(
            "UPDATE versions SET ingested = true, parser_version = $pv WHERE id = $vid",
            json!({ "vid": version_id, "pv": PARSER_VERSION }),
        )
        .await?;
        tx.commit().await
    }
}

async fn ensure_version(tx: &Tx, repo: &str, tag: &str) -> Result<i64> {
    let rows = tx
        .query(
            "WITH r AS (
             INSERT INTO repositories (name) VALUES ($repo)
             ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
             RETURNING id
         )
         INSERT INTO versions (repository_id, tag) SELECT id, $tag FROM r
         ON CONFLICT (repository_id, tag) DO UPDATE SET tag = EXCLUDED.tag
         RETURNING id",
            json!({ "repo": repo, "tag": tag }),
        )
        .await?;
    rows.first()
        .and_then(|r| r["id"].as_i64())
        .ok_or_else(|| anyhow!("could not resolve version {repo}:{tag}"))
}

fn last_by_key(items: Vec<Value>, keys: &[&str]) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut kept: Vec<Value> = items
        .into_iter()
        .rev()
        .filter(|item| {
            let identity: Vec<String> = keys
                .iter()
                .map(|k| item[*k].as_str().unwrap_or_default().to_string())
                .collect();
            seen.insert(identity)
        })
        .collect();
    kept.reverse();
    kept
}

fn content_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

async fn write_file(tx: &Tx, version_id: i64, file: &ParsedFile, history: &History) -> Result<()> {
    let rows = tx
        .query(
            "INSERT INTO files (version_id, path, language, origin) VALUES ($vid, $path, $lang, $origin)
         ON CONFLICT (version_id, path) DO UPDATE SET language = EXCLUDED.language, origin = EXCLUDED.origin
         RETURNING id",
            json!({ "vid": version_id, "path": file.path, "lang": file.language, "origin": file.origin.as_str() }),
        )
        .await?;
    let file_id = rows
        .first()
        .and_then(|r| r["id"].as_i64())
        .ok_or_else(|| anyhow!("could not insert file {}", file.path))?;

    if !file.functions.is_empty() {
        let fns = last_by_key(
            file.functions
                .iter()
                .map(|f| {
                    json!({
                        "name":       f.name,
                        "kind":       f.kind,
                        "signature":  f.signature,
                        "start_line": f.start_line,
                        "end_line":   f.end_line,
                        "source":     f.source,
                        "impl_type":  f.impl_type,
                        "docstring":  f.docstring,
                        "is_test":    f.is_test,
                    })
                })
                .collect(),
            &["name", "impl_type"],
        );
        tx.execute(
            "INSERT INTO symbols (file_id, version_id, label, name, kind, signature,
                                  start_line, end_line, source, impl_type, docstring, is_test)
             SELECT $file_id, $vid, 'Function', f->>'name', f->>'kind', f->>'signature',
                    (f->>'start_line')::int, (f->>'end_line')::int, f->>'source', f->>'impl_type',
                    f->>'docstring', coalesce((f->>'is_test')::boolean, false)
             FROM jsonb_array_elements($fns::jsonb) AS f
             ON CONFLICT (file_id, label, coalesce(impl_type, ''), name) DO UPDATE SET
                 kind = EXCLUDED.kind, signature = EXCLUDED.signature,
                 start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line,
                 source = EXCLUDED.source, impl_type = EXCLUDED.impl_type,
                 docstring = EXCLUDED.docstring, is_test = EXCLUDED.is_test",
            json!({ "file_id": file_id, "vid": version_id, "fns": fns }),
        )
        .await?;
    }

    if !file.classes.is_empty() {
        let classes = last_by_key(
            file.classes
                .iter()
                .map(|c| {
                    json!({
                        "name":       c.name,
                        "kind":       c.kind,
                        "start_line": c.start_line,
                        "end_line":   c.end_line,
                        "source":     c.source,
                        "bases":      c.bases,
                        "traits":     c.traits,
                        "embeds":     c.embeds,
                        "uses":       c.uses,
                        "docstring":  c.docstring,
                        "decorators": c.decorators,
                        "is_test":    c.is_test,
                    })
                })
                .collect(),
            &["name"],
        );
        tx.execute(
            "INSERT INTO symbols (file_id, version_id, label, name, kind, start_line, end_line,
                                  source, bases, traits, embeds, uses, docstring, decorators, is_test)
             SELECT $file_id, $vid, 'Class', c->>'name', c->>'kind',
                    (c->>'start_line')::int, (c->>'end_line')::int, c->>'source',
                    ARRAY(SELECT jsonb_array_elements_text(c->'bases')),
                    ARRAY(SELECT jsonb_array_elements_text(c->'traits')),
                    ARRAY(SELECT jsonb_array_elements_text(c->'embeds')),
                    ARRAY(SELECT jsonb_array_elements_text(c->'uses')),
                    c->>'docstring',
                    ARRAY(SELECT jsonb_array_elements_text(coalesce(c->'decorators', '[]'::jsonb))),
                    coalesce((c->>'is_test')::boolean, false)
             FROM jsonb_array_elements($classes::jsonb) AS c
             ON CONFLICT (file_id, label, coalesce(impl_type, ''), name) DO UPDATE SET
                 kind = EXCLUDED.kind, start_line = EXCLUDED.start_line,
                 end_line = EXCLUDED.end_line, source = EXCLUDED.source,
                 bases = EXCLUDED.bases, traits = EXCLUDED.traits,
                 embeds = EXCLUDED.embeds, uses = EXCLUDED.uses,
                 docstring = EXCLUDED.docstring, decorators = EXCLUDED.decorators,
                 is_test = EXCLUDED.is_test",
            json!({ "file_id": file_id, "vid": version_id, "classes": classes }),
        )
        .await?;
    }

    if !file.imports.is_empty() {
        let imports = last_by_key(
            file.imports
                .iter()
                .map(|i| json!({ "target": i.target, "line": i.line }))
                .collect(),
            &["target"],
        );
        tx.execute(
            "INSERT INTO imports (file_id, target, line)
             SELECT $file_id, i->>'target', (i->>'line')::int
             FROM jsonb_array_elements($imports::jsonb) AS i
             ON CONFLICT (file_id, target) DO UPDATE SET line = EXCLUDED.line",
            json!({ "file_id": file_id, "imports": imports }),
        )
        .await?;
    }

    if !file.units.is_empty() {
        let units: Vec<Value> = file
            .units
            .iter()
            .map(|u| {
                let mut text = u.text.clone();
                if u.kind == "module" {
                    if let Some(subjects) = history.subjects.get(&file.path) {
                        text.push_str("\nRecent changes:\n");
                        text.push_str(&subjects.join("\n"));
                    }
                }
                json!({
                    "kind": u.kind, "name": u.name, "start_line": u.start_line, "end_line": u.end_line,
                    "hash": content_hash(&text), "text": text,
                    "owner_name": u.owner.as_ref().map(|o| o.name.clone()),
                    "owner_impl": u.owner.as_ref().and_then(|o| o.impl_type.clone()),
                    "owner_line": u.owner.as_ref().map(|o| o.start_line),
                })
            })
            .collect();
        tx.execute(
            "INSERT INTO code_units (version_id, file_id, kind, name, path, language, start_line,
                                     end_line, text, content_hash, symbol_id)
             SELECT $vid, $file_id, u->>'kind', u->>'name', $path, $lang,
                    (u->>'start_line')::int, (u->>'end_line')::int, u->>'text', u->>'hash',
                    (SELECT s.id FROM symbols s
                     WHERE s.file_id = $file_id AND s.label = 'Function'
                       AND s.name = u->>'owner_name'
                       AND coalesce(s.impl_type, '') = coalesce(u->>'owner_impl', '')
                     ORDER BY abs(s.start_line - coalesce((u->>'owner_line')::int, 0))
                     LIMIT 1)
             FROM jsonb_array_elements($units::jsonb) AS u",
            json!({ "vid": version_id, "file_id": file_id, "path": file.path, "lang": file.language, "units": units }),
        )
        .await?;
    }

    Ok(())
}

const SYMBOLS_SQL: &str = r#"
SELECT s.id, f.path, s.label, s.name, s.impl_type, coalesce(s.start_line, 0) AS start_line,
       coalesce(s.bases, '{}') AS bases, coalesce(s.traits, '{}') AS traits,
       coalesce(s.embeds, '{}') AS embeds, coalesce(s.uses, '{}') AS uses,
       coalesce(s.decorators, '{}') AS decorators
FROM symbols s JOIN files f ON f.id = s.file_id
WHERE s.version_id = $vid
"#;

const UNITS_SQL: &str =
    "SELECT id, kind, name, path FROM code_units WHERE version_id = $vid AND kind <> 'chunk'";

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn symbol_record(row: &Value) -> SymbolRecord {
    SymbolRecord {
        id: row["id"].as_i64().unwrap_or(0),
        file: row["path"].as_str().unwrap_or_default().to_string(),
        label: if row["label"] == "Class" {
            Label::Class
        } else {
            Label::Function
        },
        name: row["name"].as_str().unwrap_or_default().to_string(),
        impl_type: row["impl_type"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        bases: strings(&row["bases"]),
        traits: strings(&row["traits"]),
        embeds: strings(&row["embeds"]),
        uses: strings(&row["uses"]),
        decorators: strings(&row["decorators"]),
    }
}

type CallerKey = (String, String, String, u32);

fn caller_key(file: &str, name: &str, impl_type: Option<&str>, start_line: u32) -> CallerKey {
    (
        file.to_string(),
        name.to_string(),
        impl_type.unwrap_or_default().to_string(),
        start_line,
    )
}

async fn insert_symbol_edges(tx: &Tx, edges: &[ResolvedEdge]) -> Result<()> {
    for batch in edges.chunks(EDGE_BATCH) {
        let rows: Vec<Value> = batch
            .iter()
            .map(|e| json!({ "src": e.src, "dst": e.dst, "relation": e.relation, "line": e.line }))
            .collect();
        tx.execute(
            "INSERT INTO symbol_edges (src_id, dst_id, relation, line)
             SELECT (e->>'src')::bigint, (e->>'dst')::bigint, e->>'relation', (e->>'line')::int
             FROM jsonb_array_elements($edges::jsonb) AS e
             ON CONFLICT DO NOTHING",
            json!({ "edges": rows }),
        )
        .await?;
    }
    Ok(())
}

async fn insert_unit_edges(tx: &Tx, edges: &[Value]) -> Result<()> {
    for batch in edges.chunks(EDGE_BATCH) {
        tx.execute(
            "INSERT INTO unit_edges (src_unit_id, src_symbol_id, dst_unit_id, dst_symbol_id, relation, weight)
             SELECT (e->>'src_unit')::bigint, (e->>'src_symbol')::bigint, (e->>'dst_unit')::bigint,
                    (e->>'dst_symbol')::bigint, e->>'relation', coalesce((e->>'weight')::real, 1)
             FROM jsonb_array_elements($edges::jsonb) AS e",
            json!({ "edges": batch }),
        )
        .await?;
    }
    Ok(())
}

async fn write_edges(
    tx: &Tx,
    version_id: i64,
    files: &[ParsedFile],
    history: &History,
) -> Result<()> {
    let symbols: Vec<SymbolRecord> = tx
        .query(SYMBOLS_SQL, json!({ "vid": version_id }))
        .await?
        .iter()
        .map(symbol_record)
        .collect();
    let start_lines: HashMap<i64, u32> = tx
        .query(SYMBOLS_SQL, json!({ "vid": version_id }))
        .await?
        .iter()
        .map(|r| {
            (
                r["id"].as_i64().unwrap_or(0),
                r["start_line"].as_i64().unwrap_or(0) as u32,
            )
        })
        .collect();
    let mut callers: HashMap<CallerKey, i64> = HashMap::new();
    let mut by_name: HashMap<(String, String, String), Vec<i64>> = HashMap::new();
    for s in symbols.iter().filter(|s| s.label == Label::Function) {
        let line = start_lines.get(&s.id).copied().unwrap_or(0);
        callers.insert(
            caller_key(&s.file, &s.name, s.impl_type.as_deref(), line),
            s.id,
        );
        by_name
            .entry((
                s.file.clone(),
                s.name.clone(),
                s.impl_type.clone().unwrap_or_default(),
            ))
            .or_default()
            .push(s.id);
    }
    let locate = |file: &str, name: &str, impl_type: Option<&str>, line: u32| -> Option<i64> {
        callers
            .get(&caller_key(file, name, impl_type, line))
            .copied()
            .or_else(|| {
                by_name
                    .get(&(
                        file.to_string(),
                        name.to_string(),
                        impl_type.unwrap_or_default().to_string(),
                    ))
                    .and_then(|ids| ids.first().copied())
            })
    };
    let mut calls = Vec::new();
    let mut references = Vec::new();
    let mut config_reads: Vec<(i64, String)> = Vec::new();
    let imports: HashMap<String, Vec<String>> = files
        .iter()
        .map(|f| {
            (
                f.path.clone(),
                f.imports.iter().map(|i| i.target.clone()).collect(),
            )
        })
        .collect();
    for file in files {
        for function in &file.functions {
            let Some(caller) = locate(
                &file.path,
                &function.name,
                function.impl_type.as_deref(),
                function.start_line,
            ) else {
                continue;
            };
            for call in &function.calls {
                calls.push(CallRecord {
                    caller,
                    callee: call.callee.clone(),
                    line: call.line,
                });
            }
        }
        for reference in &file.references {
            let Some(from) = locate(
                &file.path,
                &reference.from.name,
                reference.from.impl_type.as_deref(),
                reference.from.start_line,
            ) else {
                continue;
            };
            match reference.relation.as_str() {
                "RAISES" => references.push(ReferenceRecord {
                    from,
                    target: reference.target.clone(),
                    kind: RefKind::Raises,
                    line: reference.line,
                }),
                "REGISTERS" => references.push(ReferenceRecord {
                    from,
                    target: reference.target.clone(),
                    kind: RefKind::Registers,
                    line: reference.line,
                }),
                "READS_CONFIG" => config_reads.push((from, reference.target.clone())),
                _ => {}
            }
        }
    }
    let resolver = Resolver::new(&symbols, &imports);
    insert_symbol_edges(tx, &resolver.edges(&calls, &references)).await?;

    let units = tx.query(UNITS_SQL, json!({ "vid": version_id })).await?;
    let mut config_units: HashMap<String, i64> = HashMap::new();
    let mut module_units: HashMap<String, i64> = HashMap::new();
    let mut doc_units: Vec<(i64, String, String)> = Vec::new();
    for unit in &units {
        let id = unit["id"].as_i64().unwrap_or(0);
        let name = unit["name"].as_str().unwrap_or_default().to_string();
        let path = unit["path"].as_str().unwrap_or_default().to_string();
        match unit["kind"].as_str() {
            Some("config") => {
                config_units.entry(name).or_insert(id);
            }
            Some("module") => {
                module_units.insert(path, id);
            }
            Some("doc") => doc_units.push((id, name, path)),
            _ => {}
        }
    }
    let mut unit_edges: Vec<Value> = Vec::new();
    let mut seen: HashSet<(i64, String)> = HashSet::new();
    for (from, key) in config_reads {
        if let Some(unit) = config_units.get(&key) {
            if seen.insert((from, key.clone())) {
                unit_edges.push(
                    json!({ "src_symbol": from, "dst_unit": unit, "relation": "READS_CONFIG" }),
                );
            }
        }
    }
    let unit_id_by_file_and_name: HashMap<(String, String), i64> = units
        .iter()
        .map(|u| {
            (
                (
                    u["path"].as_str().unwrap_or_default().to_string(),
                    u["name"].as_str().unwrap_or_default().to_string(),
                ),
                u["id"].as_i64().unwrap_or(0),
            )
        })
        .collect();
    let pseudo = |path: &str| SymbolRecord {
        id: 0,
        file: path.to_string(),
        label: Label::Function,
        name: String::new(),
        impl_type: None,
        bases: vec![],
        traits: vec![],
        embeds: vec![],
        uses: vec![],
        decorators: vec![],
    };
    for file in files {
        for unit in &file.units {
            let Some(unit_id) =
                unit_id_by_file_and_name.get(&(file.path.clone(), unit.name.clone()))
            else {
                continue;
            };
            for mention in &unit.mentions {
                let from = pseudo(&file.path);
                let target = resolver
                    .class_named(&from, mention)
                    .or_else(|| resolver.callee(&from, mention).map(|(t, _)| t));
                if let Some(target) = target {
                    unit_edges.push(json!({ "src_unit": unit_id, "dst_symbol": symbols[target].id, "relation": "DOCUMENTS" }));
                }
            }
            for class in &unit.registers {
                if let Some(target) = resolver.class_named(&pseudo(&file.path), class) {
                    unit_edges.push(json!({ "src_unit": unit_id, "dst_symbol": symbols[target].id, "relation": "REGISTERS" }));
                }
            }
        }
    }
    for (a, b, weight) in &history.co_changes {
        if let (Some(x), Some(y)) = (module_units.get(a), module_units.get(b)) {
            unit_edges.push(
                json!({ "src_unit": x, "dst_unit": y, "relation": "CO_CHANGES", "weight": weight }),
            );
            unit_edges.push(
                json!({ "src_unit": y, "dst_unit": x, "relation": "CO_CHANGES", "weight": weight }),
            );
        }
    }
    let _ = &doc_units;
    insert_unit_edges(tx, &unit_edges).await
}
