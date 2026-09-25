use std::collections::HashSet;

use anyhow::{anyhow, Result};
use harvest_db::{Db, Tx};
use serde_json::{json, Value};

use super::model::ParsedFile;

const CLASS_LINKS: &[(&str, &str)] = &[
    ("bases", "INHERITS"),
    ("traits", "IMPLEMENTS"),
    ("uses", "USES"),
    ("embeds", "EMBEDS"),
];

pub struct GraphWriter {
    db: Db,
}

impl GraphWriter {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn reset_ingested(&self) -> Result<()> {
        self.db.execute("UPDATE versions SET ingested = false", json!({})).await?;
        Ok(())
    }

    pub async fn is_ingested(&self, repo: &str, tag: &str) -> Result<bool> {
        let rows = self.db.query(
            "SELECT 1 AS ok FROM code_versions WHERE repo = $repo AND tag = $tag AND ingested LIMIT 1",
            json!({ "repo": repo, "tag": tag }),
        ).await?;
        Ok(!rows.is_empty())
    }

    pub async fn ingested_versions(&self, repo: &str) -> Result<Vec<String>> {
        let rows = self.db.query(
            "SELECT tag FROM code_versions WHERE repo = $repo AND ingested ORDER BY timestamp",
            json!({ "repo": repo }),
        ).await?;
        Ok(rows.iter().filter_map(|r| r["tag"].as_str().map(String::from)).collect())
    }

    pub async fn upsert_repository(&self, name: &str, url: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO repositories (name, url) VALUES ($name, $url)
             ON CONFLICT (name) DO UPDATE SET url = EXCLUDED.url",
            json!({ "name": name, "url": url }),
        ).await?;
        Ok(())
    }

    pub async fn upsert_version(&self, repo: &str, tag: &str, timestamp: i64, ingested: bool) -> Result<()> {
        self.db.execute(
            "WITH r AS (
                 INSERT INTO repositories (name) VALUES ($repo)
                 ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
                 RETURNING id
             )
             INSERT INTO versions (repository_id, tag, timestamp, ingested)
             SELECT id, $tag, $timestamp::bigint, $ingested::boolean FROM r
             ON CONFLICT (repository_id, tag)
                 DO UPDATE SET timestamp = EXCLUDED.timestamp, ingested = EXCLUDED.ingested",
            json!({ "repo": repo, "tag": tag, "timestamp": timestamp, "ingested": ingested }),
        ).await?;
        Ok(())
    }

    pub async fn write_version(&self, repo: &str, tag: &str, files: &[ParsedFile]) -> Result<()> {
        let tx = self.db.begin().await?;
        let version_id = ensure_version(&tx, repo, tag).await?;
        tx.execute("DELETE FROM files WHERE version_id = $vid", json!({ "vid": version_id })).await?;
        for file in files {
            write_file(&tx, version_id, file).await?;
        }
        for file in files {
            write_call_edges(&tx, version_id, file).await?;
        }
        for (column, relation) in CLASS_LINKS {
            link_class_edges(&tx, version_id, column, relation).await?;
        }
        tx.execute("UPDATE versions SET ingested = true WHERE id = $vid", json!({ "vid": version_id })).await?;
        tx.commit().await
    }
}

async fn ensure_version(tx: &Tx, repo: &str, tag: &str) -> Result<i64> {
    let rows = tx.query(
        "WITH r AS (
             INSERT INTO repositories (name) VALUES ($repo)
             ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
             RETURNING id
         )
         INSERT INTO versions (repository_id, tag) SELECT id, $tag FROM r
         ON CONFLICT (repository_id, tag) DO UPDATE SET tag = EXCLUDED.tag
         RETURNING id",
        json!({ "repo": repo, "tag": tag }),
    ).await?;
    rows.first()
        .and_then(|r| r["id"].as_i64())
        .ok_or_else(|| anyhow!("could not resolve version {repo}:{tag}"))
}

/// Keeps the last entry for each key, mirroring how repeated names overwrite one another.
fn last_by_key(items: Vec<Value>, key: &str) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut kept: Vec<Value> = items.into_iter().rev()
        .filter(|item| seen.insert(item[key].as_str().unwrap_or_default().to_string()))
        .collect();
    kept.reverse();
    kept
}

async fn write_file(tx: &Tx, version_id: i64, file: &ParsedFile) -> Result<()> {
    let rows = tx.query(
        "INSERT INTO files (version_id, path, language) VALUES ($vid, $path, $lang)
         ON CONFLICT (version_id, path) DO UPDATE SET language = EXCLUDED.language
         RETURNING id",
        json!({ "vid": version_id, "path": file.path, "lang": file.language }),
    ).await?;
    let file_id = rows.first()
        .and_then(|r| r["id"].as_i64())
        .ok_or_else(|| anyhow!("could not insert file {}", file.path))?;

    if !file.functions.is_empty() {
        let fns = last_by_key(file.functions.iter().map(|f| json!({
            "name":       f.name,
            "kind":       f.kind,
            "signature":  f.signature,
            "start_line": f.start_line,
            "end_line":   f.end_line,
            "source":     f.source,
            "impl_type":  f.impl_type,
        })).collect(), "name");
        tx.execute(
            "INSERT INTO symbols (file_id, version_id, label, name, kind, signature,
                                  start_line, end_line, source, impl_type)
             SELECT $file_id, $vid, 'Function', f->>'name', f->>'kind', f->>'signature',
                    (f->>'start_line')::int, (f->>'end_line')::int, f->>'source', f->>'impl_type'
             FROM jsonb_array_elements($fns::jsonb) AS f
             ON CONFLICT (file_id, label, name) DO UPDATE SET
                 kind = EXCLUDED.kind, signature = EXCLUDED.signature,
                 start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line,
                 source = EXCLUDED.source, impl_type = EXCLUDED.impl_type",
            json!({ "file_id": file_id, "vid": version_id, "fns": fns }),
        ).await?;
    }

    if !file.classes.is_empty() {
        let classes = last_by_key(file.classes.iter().map(|c| json!({
            "name":       c.name,
            "kind":       c.kind,
            "start_line": c.start_line,
            "end_line":   c.end_line,
            "source":     c.source,
            "bases":      c.bases,
            "traits":     c.traits,
            "embeds":     c.embeds,
            "uses":       c.uses,
        })).collect(), "name");
        tx.execute(
            "INSERT INTO symbols (file_id, version_id, label, name, kind, start_line, end_line,
                                  source, bases, traits, embeds, uses)
             SELECT $file_id, $vid, 'Class', c->>'name', c->>'kind',
                    (c->>'start_line')::int, (c->>'end_line')::int, c->>'source',
                    ARRAY(SELECT jsonb_array_elements_text(c->'bases')),
                    ARRAY(SELECT jsonb_array_elements_text(c->'traits')),
                    ARRAY(SELECT jsonb_array_elements_text(c->'embeds')),
                    ARRAY(SELECT jsonb_array_elements_text(c->'uses'))
             FROM jsonb_array_elements($classes::jsonb) AS c
             ON CONFLICT (file_id, label, name) DO UPDATE SET
                 kind = EXCLUDED.kind, start_line = EXCLUDED.start_line,
                 end_line = EXCLUDED.end_line, source = EXCLUDED.source,
                 bases = EXCLUDED.bases, traits = EXCLUDED.traits,
                 embeds = EXCLUDED.embeds, uses = EXCLUDED.uses",
            json!({ "file_id": file_id, "vid": version_id, "classes": classes }),
        ).await?;
    }

    if !file.imports.is_empty() {
        let imports = last_by_key(file.imports.iter()
            .map(|i| json!({ "target": i.target, "line": i.line }))
            .collect(), "target");
        tx.execute(
            "INSERT INTO imports (file_id, target, line)
             SELECT $file_id, i->>'target', (i->>'line')::int
             FROM jsonb_array_elements($imports::jsonb) AS i
             ON CONFLICT (file_id, target) DO UPDATE SET line = EXCLUDED.line",
            json!({ "file_id": file_id, "imports": imports }),
        ).await?;
    }

    Ok(())
}

async fn write_call_edges(tx: &Tx, version_id: i64, file: &ParsedFile) -> Result<()> {
    let fn_calls: Vec<Value> = file.functions.iter()
        .filter(|f| !f.calls.is_empty())
        .map(|f| json!({
            "caller":      f.name,
            "caller_file": f.file,
            "calls": f.calls.iter().map(|c| json!({ "callee": c.callee, "line": c.line })).collect::<Vec<_>>(),
        }))
        .collect();
    if fn_calls.is_empty() {
        return Ok(());
    }
    tx.execute(
        "WITH calls AS (
             SELECT fc->>'caller' AS caller, fc->>'caller_file' AS caller_file,
                    c->>'callee' AS callee, (c->>'line')::int AS line
             FROM jsonb_array_elements($fn_calls::jsonb) AS fc,
                  jsonb_array_elements(fc->'calls') AS c
         ), resolved AS (
             SELECT cs.id AS src_id, calls.line,
                    COALESCE(same.id, CASE WHEN any_file.n = 1 THEN any_file.only_id END) AS dst_id
             FROM calls
             JOIN files cf   ON cf.version_id = $vid AND cf.path = calls.caller_file
             JOIN symbols cs ON cs.file_id = cf.id AND cs.label = 'Function' AND cs.name = calls.caller
             LEFT JOIN symbols same
                    ON same.file_id = cf.id AND same.label = 'Function' AND same.name = calls.callee
             CROSS JOIN LATERAL (
                 SELECT count(*) AS n, min(s.id) AS only_id FROM symbols s
                 WHERE s.version_id = $vid AND s.label = 'Function' AND s.name = calls.callee
             ) any_file
         )
         INSERT INTO symbol_edges (src_id, dst_id, relation, line)
         SELECT src_id, dst_id, 'CALLS', line FROM resolved
         WHERE dst_id IS NOT NULL AND dst_id <> src_id
         ON CONFLICT DO NOTHING",
        json!({ "vid": version_id, "fn_calls": fn_calls }),
    ).await?;
    Ok(())
}

async fn link_class_edges(tx: &Tx, version_id: i64, column: &str, relation: &str) -> Result<()> {
    tx.execute(
        &format!(
            "INSERT INTO symbol_edges (src_id, dst_id, relation)
             SELECT child.id, min(target.id), '{relation}'
             FROM symbols child
             CROSS JOIN LATERAL unnest(child.{column}) AS target_name
             JOIN symbols target
               ON target.version_id = child.version_id
              AND target.label = 'Class' AND target.name = target_name
             WHERE child.version_id = $vid AND child.label = 'Class'
             GROUP BY child.id, target_name
             HAVING count(*) = 1
             ON CONFLICT DO NOTHING"
        ),
        json!({ "vid": version_id }),
    ).await?;
    Ok(())
}
