use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::LlmProviderConfig;
use crate::llm::pricing::{format_cost, price, ModelPricing};
use crate::llm::types::{Usage, UsedProvider};
use crate::neo4j::Neo4jClient;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CostScope {
    Chat,
    Design,
    Provision,
    Proposal,
    Title,
    Overview,
    HarvesterDoc,
}

impl CostScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Design => "design",
            Self::Provision => "provision",
            Self::Proposal => "proposal",
            Self::Title => "title",
            Self::Overview => "overview",
            Self::HarvesterDoc => "harvester_doc",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmCallRecord {
    pub id: String,
    pub scope: CostScope,
    pub turn_id: String,
    pub project_id: Option<String>,
    pub conversation_id: Option<String>,
    pub deployment_id: Option<String>,
    pub proposal_id: Option<String>,
    pub artifact_id: Option<String>,
    pub user_id: String,
    pub provider_id: String,
    pub kind: String,
    pub model: String,
    pub usage: Usage,
    pub cost_microusd: i64,
    pub duration_ms: u64,
    pub iteration_index: Option<usize>,
    pub llm_call_count: usize,
    pub succeeded: bool,
    pub created_at: String,
}

fn default_pricing_for(kind: &str, model: &str) -> Option<ModelPricing> {
    let m = model.to_lowercase();
    let k = kind.to_lowercase();
    if k.contains("anthropic") || m.contains("claude") {
        if m.contains("sonnet-4") || m.contains("sonnet-4-6") || m.contains("claude-sonnet-4") {
            return Some(ModelPricing { input_per_1k: 3000, cache_read_per_1k: 300, cache_creation_per_1k: 3750, output_per_1k: 15000, reasoning_per_1k: 0 });
        }
        if m.contains("opus") {
            return Some(ModelPricing { input_per_1k: 15000, cache_read_per_1k: 1500, cache_creation_per_1k: 18750, output_per_1k: 75000, reasoning_per_1k: 0 });
        }
        if m.contains("haiku") {
            return Some(ModelPricing { input_per_1k: 100, cache_read_per_1k: 10, cache_creation_per_1k: 125, output_per_1k: 500, reasoning_per_1k: 0 });
        }
        return Some(ModelPricing { input_per_1k: 3000, cache_read_per_1k: 300, cache_creation_per_1k: 3750, output_per_1k: 15000, reasoning_per_1k: 0 });
    }
    if k.contains("gemini") || m.contains("gemini") {
        if m.contains("2.5-pro") || m.contains("3") {
            return Some(ModelPricing { input_per_1k: 1250, cache_read_per_1k: 312, cache_creation_per_1k: 0, output_per_1k: 5000, reasoning_per_1k: 5000 });
        }
        if m.contains("flash") {
            return Some(ModelPricing { input_per_1k: 300, cache_read_per_1k: 75, cache_creation_per_1k: 0, output_per_1k: 2500, reasoning_per_1k: 2500 });
        }
        return Some(ModelPricing { input_per_1k: 1250, cache_read_per_1k: 312, cache_creation_per_1k: 0, output_per_1k: 5000, reasoning_per_1k: 5000 });
    }
    if k.contains("openai") || m.contains("gpt") {
        if m.contains("gpt-4o") {
            return Some(ModelPricing { input_per_1k: 2500, cache_read_per_1k: 1250, cache_creation_per_1k: 0, output_per_1k: 10000, reasoning_per_1k: 10000 });
        }
        if m.contains("gpt-4") {
            return Some(ModelPricing { input_per_1k: 30000, cache_read_per_1k: 1500, cache_creation_per_1k: 0, output_per_1k: 60000, reasoning_per_1k: 0 });
        }
        if m.contains("o1") || m.contains("o3") || m.contains("o4") {
            return Some(ModelPricing { input_per_1k: 15000, cache_read_per_1k: 7500, cache_creation_per_1k: 0, output_per_1k: 60000, reasoning_per_1k: 60000 });
        }
    }
    if m.contains("kimi") || m.contains("k3") {
        return Some(ModelPricing { input_per_1k: 600, cache_read_per_1k: 60, cache_creation_per_1k: 0, output_per_1k: 2200, reasoning_per_1k: 0 });
    }
    if m.contains("deepseek") {
        return Some(ModelPricing { input_per_1k: 270, cache_read_per_1k: 27, cache_creation_per_1k: 0, output_per_1k: 1100, reasoning_per_1k: 0 });
    }
    if m.contains("llama") || m.contains("mistral") || m.contains("ministral") {
        return Some(ModelPricing { input_per_1k: 200, cache_read_per_1k: 20, cache_creation_per_1k: 0, output_per_1k: 600, reasoning_per_1k: 0 });
    }
    Some(ModelPricing { input_per_1k: 1000, cache_read_per_1k: 100, cache_creation_per_1k: 0, output_per_1k: 3000, reasoning_per_1k: 0 })
}

#[derive(Clone, Default)]
pub struct PricingTable {
    entries: HashMap<(String, String), ModelPricing>,
}

impl PricingTable {
    pub fn from_configs(configs: &[LlmProviderConfig]) -> Self {
        let mut entries = HashMap::new();
        for c in configs {
            if let Some(pc) = c.pricing() {
                entries.insert((c.kind().to_string(), c.model().to_string()), pc.to_pricing());
            } else if let Some(dp) = default_pricing_for(c.kind(), c.model()) {
                entries.insert((c.kind().to_string(), c.model().to_string()), dp);
            }
        }
        Self { entries }
    }

    pub fn lookup(&self, kind: &str, model: &str) -> Option<&ModelPricing> {
        self.entries.get(&(kind.to_string(), model.to_string()))
    }

    pub fn price_call(&self, usage: &Usage, kind: &str, model: &str) -> i64 {
        let pricing = match self.lookup(kind, model) {
            Some(p) => Some(p.clone()),
            None => default_pricing_for(kind, model),
        };
        price(usage, pricing.as_ref())
    }
}

pub type SharedPricingTable = Arc<PricingTable>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CostSummary {
    pub total_microusd: i64,
    pub total_calls: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_reasoning_tokens: u64,
}

impl CostSummary {
    pub fn from_usage_and_cost(usage: &Usage, cost: i64, calls: usize) -> Self {
        Self {
            total_microusd: cost,
            total_calls: calls,
            total_input_tokens: usage.input_tokens,
            total_output_tokens: usage.output_tokens,
            total_cache_read_tokens: usage.cache_read_tokens,
            total_cache_creation_tokens: usage.cache_creation_tokens,
            total_reasoning_tokens: usage.reasoning_tokens,
        }
    }

    pub fn add(&mut self, other: &CostSummary) {
        self.total_microusd += other.total_microusd;
        self.total_calls += other.total_calls;
        self.total_input_tokens += other.total_input_tokens;
        self.total_output_tokens += other.total_output_tokens;
        self.total_cache_read_tokens += other.total_cache_read_tokens;
        self.total_cache_creation_tokens += other.total_cache_creation_tokens;
        self.total_reasoning_tokens += other.total_reasoning_tokens;
    }

    pub fn formatted_cost(&self) -> String {
        format_cost(self.total_microusd)
    }
}

pub fn build_record(
    scope: CostScope,
    turn_id: &str,
    user_id: &str,
    used: &UsedProvider,
    usage: Usage,
    pricing: &PricingTable,
    duration_ms: u64,
    iteration_index: Option<usize>,
    succeeded: bool,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    deployment_id: Option<&str>,
    proposal_id: Option<&str>,
    artifact_id: Option<&str>,
) -> LlmCallRecord {
    let cost_microusd = pricing.price_call(&usage, &used.kind, &used.model);
    LlmCallRecord {
        id: uuid::Uuid::new_v4().to_string(),
        scope,
        turn_id: turn_id.to_string(),
        project_id: project_id.map(String::from),
        conversation_id: conversation_id.map(String::from),
        deployment_id: deployment_id.map(String::from),
        proposal_id: proposal_id.map(String::from),
        artifact_id: artifact_id.map(String::from),
        user_id: user_id.to_string(),
        provider_id: used.provider_id.clone(),
        kind: used.kind.clone(),
        model: used.model.clone(),
        usage: usage.clone(),
        cost_microusd,
        duration_ms,
        iteration_index,
        llm_call_count: 1,
        succeeded,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

pub fn build_turn_record(
    scope: CostScope,
    turn_id: &str,
    user_id: &str,
    used: Option<&UsedProvider>,
    usage: &Usage,
    llm_call_count: usize,
    pricing: &PricingTable,
    duration_ms: u64,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    deployment_id: Option<&str>,
    proposal_id: Option<&str>,
    artifact_id: Option<&str>,
) -> Option<LlmCallRecord> {
    let used = used?;
    if llm_call_count == 0 { return None; }
    let cost_microusd = pricing.price_call(usage, &used.kind, &used.model);
    Some(LlmCallRecord {
        id: uuid::Uuid::new_v4().to_string(),
        scope,
        turn_id: turn_id.to_string(),
        project_id: project_id.map(String::from),
        conversation_id: conversation_id.map(String::from),
        deployment_id: deployment_id.map(String::from),
        proposal_id: proposal_id.map(String::from),
        artifact_id: artifact_id.map(String::from),
        user_id: user_id.to_string(),
        provider_id: used.provider_id.clone(),
        kind: used.kind.clone(),
        model: used.model.clone(),
        usage: usage.clone(),
        cost_microusd,
        duration_ms,
        iteration_index: None,
        llm_call_count,
        succeeded: true,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn setup_constraints(neo4j: &Neo4jClient) -> Result<()> {
    neo4j.run("CREATE CONSTRAINT llm_call_id IF NOT EXISTS FOR (c:LlmCall) REQUIRE c.id IS UNIQUE").await?;
    Ok(())
}

pub async fn record_llm_call(neo4j: &Neo4jClient, record: &LlmCallRecord) -> Result<()> {
    let params = json!({
        "id": record.id,
        "scope": record.scope.as_str(),
        "turn_id": record.turn_id,
        "project_id": record.project_id,
        "conversation_id": record.conversation_id,
        "deployment_id": record.deployment_id,
        "proposal_id": record.proposal_id,
        "artifact_id": record.artifact_id,
        "user_id": record.user_id,
        "provider_id": record.provider_id,
        "kind": record.kind,
        "model": record.model,
        "input_tokens": record.usage.input_tokens as i64,
        "output_tokens": record.usage.output_tokens as i64,
        "cache_read_tokens": record.usage.cache_read_tokens as i64,
        "cache_creation_tokens": record.usage.cache_creation_tokens as i64,
        "reasoning_tokens": record.usage.reasoning_tokens as i64,
        "cost_microusd": record.cost_microusd,
        "duration_ms": record.duration_ms as i64,
        "iteration_index": record.iteration_index.map(|i| i as i64),
        "llm_call_count": record.llm_call_count as i64,
        "succeeded": record.succeeded,
        "created_at": record.created_at,
    });

    let cypher = r#"
        CREATE (c:LlmCall {
            id: $id, scope: $scope, turn_id: $turn_id,
            project_id: $project_id, conversation_id: $conversation_id,
            deployment_id: $deployment_id, proposal_id: $proposal_id, artifact_id: $artifact_id,
            user_id: $user_id, provider_id: $provider_id, kind: $kind, model: $model,
            input_tokens: $input_tokens, output_tokens: $output_tokens,
            cache_read_tokens: $cache_read_tokens, cache_creation_tokens: $cache_creation_tokens,
            reasoning_tokens: $reasoning_tokens,
            cost_microusd: $cost_microusd, duration_ms: $duration_ms,
            iteration_index: $iteration_index, llm_call_count: $llm_call_count,
            succeeded: $succeeded, created_at: $created_at
        })
        WITH c
        CALL { WITH c OPTIONAL MATCH (p:Project {id: $project_id}) RETURN p LIMIT 1 }
        CALL { WITH c OPTIONAL MATCH (conv:Conversation {id: $conversation_id}) RETURN conv LIMIT 1 }
        CALL { WITH c OPTIONAL MATCH (d:Deployment {id: $deployment_id}) RETURN d LIMIT 1 }
        CALL { WITH c OPTIONAL MATCH (prop:Proposal {id: $proposal_id}) RETURN prop LIMIT 1 }
        CALL { WITH c OPTIONAL MATCH (a:Artifact {id: $artifact_id}) RETURN a LIMIT 1 }
        CALL { WITH c OPTIONAL MATCH (u:User {id: $user_id}) RETURN u LIMIT 1 }
        FOREACH (_ IN CASE WHEN p IS NOT NULL THEN [1] ELSE [] END | CREATE (p)-[:INCURRED]->(c))
        FOREACH (_ IN CASE WHEN conv IS NOT NULL THEN [1] ELSE [] END | CREATE (conv)-[:INCURRED]->(c))
        FOREACH (_ IN CASE WHEN d IS NOT NULL THEN [1] ELSE [] END | CREATE (d)-[:INCURRED]->(c))
        FOREACH (_ IN CASE WHEN prop IS NOT NULL THEN [1] ELSE [] END | CREATE (prop)-[:INCURRED]->(c))
        FOREACH (_ IN CASE WHEN a IS NOT NULL THEN [1] ELSE [] END | CREATE (a)-[:INCURRED]->(c))
        FOREACH (_ IN CASE WHEN u IS NOT NULL THEN [1] ELSE [] END | CREATE (u)-[:TRIGGERED]->(c))
    "#;

    neo4j.run_with_params(cypher, params).await
}

pub async fn record_llm_calls(neo4j: &Neo4jClient, records: &[LlmCallRecord]) -> Result<()> {
    for record in records {
        if let Err(e) = record_llm_call(neo4j, record).await {
            tracing::warn!(error = %e, "failed to record LlmCall cost entry");
        }
    }
    Ok(())
}

pub async fn record_deployment_turn(
    neo4j: &Neo4jClient,
    pricing: &PricingTable,
    scope: CostScope,
    turn_id: &str,
    user_id: &str,
    project_id: &str,
    deployment_id: &str,
    proposal_id: Option<&str>,
    artifact_id: Option<&str>,
    provider_used: Option<&UsedProvider>,
    usage: &Usage,
    llm_call_count: usize,
    duration_ms: u64,
) {
    if let Some(record) = build_turn_record(
        scope, turn_id, user_id, provider_used, usage, llm_call_count, pricing,
        duration_ms, Some(project_id), None, Some(deployment_id), proposal_id, artifact_id,
    ) {
        let _ = record_llm_call(neo4j, &record).await;
    }
}

fn row_to_summary(row: &Value) -> CostSummary {
    CostSummary {
        total_microusd: row["total_cost"].as_i64().unwrap_or(0),
        total_calls: row["call_count"].as_i64().unwrap_or(0) as usize,
        total_input_tokens: row["input_tokens"].as_i64().unwrap_or(0) as u64,
        total_output_tokens: row["output_tokens"].as_i64().unwrap_or(0) as u64,
        total_cache_read_tokens: row["cache_read_tokens"].as_i64().unwrap_or(0) as u64,
        total_cache_creation_tokens: row["cache_creation_tokens"].as_i64().unwrap_or(0) as u64,
        total_reasoning_tokens: row["reasoning_tokens"].as_i64().unwrap_or(0) as u64,
    }
}

pub async fn project_cost_summary(neo4j: &Neo4jClient, project_id: &str) -> Result<CostSummary> {
    let rows = neo4j.query_read(
        "MATCH (p:Project {id: $pid})-[:INCURRED]->(c:LlmCall)
         RETURN coalesce(sum(c.cost_microusd), 0) AS total_cost,
                count(c) AS call_count,
                coalesce(sum(c.input_tokens), 0) AS input_tokens,
                coalesce(sum(c.output_tokens), 0) AS output_tokens,
                coalesce(sum(c.cache_read_tokens), 0) AS cache_read_tokens,
                coalesce(sum(c.cache_creation_tokens), 0) AS cache_creation_tokens,
                coalesce(sum(c.reasoning_tokens), 0) AS reasoning_tokens",
        json!({ "pid": project_id }),
    ).await?;
    Ok(rows.first().map(|r| row_to_summary(r)).unwrap_or_default())
}

pub async fn project_cost_by_scope(neo4j: &Neo4jClient, project_id: &str) -> Result<HashMap<String, CostSummary>> {
    let rows = neo4j.query_read(
        "MATCH (p:Project {id: $pid})-[:INCURRED]->(c:LlmCall)
         RETURN c.scope AS scope,
                coalesce(sum(c.cost_microusd), 0) AS total_cost,
                count(c) AS call_count,
                coalesce(sum(c.input_tokens), 0) AS input_tokens,
                coalesce(sum(c.output_tokens), 0) AS output_tokens,
                coalesce(sum(c.cache_read_tokens), 0) AS cache_read_tokens,
                coalesce(sum(c.cache_creation_tokens), 0) AS cache_creation_tokens,
                coalesce(sum(c.reasoning_tokens), 0) AS reasoning_tokens",
        json!({ "pid": project_id }),
    ).await?;
    let mut map = HashMap::new();
    for row in &rows {
        let scope = row["scope"].as_str().unwrap_or("unknown").to_string();
        map.insert(scope, row_to_summary(row));
    }
    Ok(map)
}

pub async fn conversation_cost_summary(neo4j: &Neo4jClient, conversation_id: &str) -> Result<CostSummary> {
    let rows = neo4j.query_read(
        "MATCH (conv:Conversation {id: $cid})-[:INCURRED]->(c:LlmCall)
         RETURN coalesce(sum(c.cost_microusd), 0) AS total_cost,
                count(c) AS call_count,
                coalesce(sum(c.input_tokens), 0) AS input_tokens,
                coalesce(sum(c.output_tokens), 0) AS output_tokens,
                coalesce(sum(c.cache_read_tokens), 0) AS cache_read_tokens,
                coalesce(sum(c.cache_creation_tokens), 0) AS cache_creation_tokens,
                coalesce(sum(c.reasoning_tokens), 0) AS reasoning_tokens",
        json!({ "cid": conversation_id }),
    ).await?;
    Ok(rows.first().map(|r| row_to_summary(r)).unwrap_or_default())
}

pub async fn conversation_cost_by_turn(neo4j: &Neo4jClient, conversation_id: &str) -> Result<Vec<(String, CostSummary)>> {
    let rows = neo4j.query_read(
        "MATCH (conv:Conversation {id: $cid})-[:INCURRED]->(c:LlmCall)
         RETURN c.turn_id AS turn_id,
                coalesce(sum(c.cost_microusd), 0) AS total_cost,
                count(c) AS call_count,
                coalesce(sum(c.input_tokens), 0) AS input_tokens,
                coalesce(sum(c.output_tokens), 0) AS output_tokens,
                coalesce(sum(c.cache_read_tokens), 0) AS cache_read_tokens,
                coalesce(sum(c.cache_creation_tokens), 0) AS cache_creation_tokens,
                coalesce(sum(c.reasoning_tokens), 0) AS reasoning_tokens
         ORDER BY min(c.created_at)",
        json!({ "cid": conversation_id }),
    ).await?;
    Ok(rows.iter().map(|r| {
        let turn_id = r["turn_id"].as_str().unwrap_or("").to_string();
        (turn_id, row_to_summary(r))
    }).collect())
}

pub async fn deployment_cost_summary(neo4j: &Neo4jClient, deployment_id: &str) -> Result<HashMap<String, CostSummary>> {
    let rows = neo4j.query_read(
        "MATCH (d:Deployment {id: $did})-[:INCURRED]->(c:LlmCall)
         RETURN c.scope AS scope,
                coalesce(sum(c.cost_microusd), 0) AS total_cost,
                count(c) AS call_count,
                coalesce(sum(c.input_tokens), 0) AS input_tokens,
                coalesce(sum(c.output_tokens), 0) AS output_tokens,
                coalesce(sum(c.cache_read_tokens), 0) AS cache_read_tokens,
                coalesce(sum(c.cache_creation_tokens), 0) AS cache_creation_tokens,
                coalesce(sum(c.reasoning_tokens), 0) AS reasoning_tokens",
        json!({ "did": deployment_id }),
    ).await?;
    let mut map = HashMap::new();
    for row in &rows {
        let scope = row["scope"].as_str().unwrap_or("unknown").to_string();
        map.insert(scope, row_to_summary(row));
    }
    Ok(map)
}

pub async fn project_cost_by_model(neo4j: &Neo4jClient, project_id: &str) -> Result<Vec<(String, String, CostSummary)>> {
    let rows = neo4j.query_read(
        "MATCH (p:Project {id: $pid})-[:INCURRED]->(c:LlmCall)
         RETURN c.kind AS kind, c.model AS model,
                coalesce(sum(c.cost_microusd), 0) AS total_cost,
                count(c) AS call_count,
                coalesce(sum(c.input_tokens), 0) AS input_tokens,
                coalesce(sum(c.output_tokens), 0) AS output_tokens,
                coalesce(sum(c.cache_read_tokens), 0) AS cache_read_tokens,
                coalesce(sum(c.cache_creation_tokens), 0) AS cache_creation_tokens,
                coalesce(sum(c.reasoning_tokens), 0) AS reasoning_tokens
         ORDER BY total_cost DESC",
        json!({ "pid": project_id }),
    ).await?;
    Ok(rows.iter().map(|r| {
        let kind = r["kind"].as_str().unwrap_or("").to_string();
        let model = r["model"].as_str().unwrap_or("").to_string();
        (kind, model, row_to_summary(r))
    }).collect())
}

pub async fn project_cost_by_user(neo4j: &Neo4jClient, project_id: &str) -> Result<Vec<(String, CostSummary)>> {
    let rows = neo4j.query_read(
        "MATCH (p:Project {id: $pid})-[:INCURRED]->(c:LlmCall)
         RETURN c.user_id AS user_id,
                coalesce(sum(c.cost_microusd), 0) AS total_cost,
                count(c) AS call_count,
                coalesce(sum(c.input_tokens), 0) AS input_tokens,
                coalesce(sum(c.output_tokens), 0) AS output_tokens,
                coalesce(sum(c.cache_read_tokens), 0) AS cache_read_tokens,
                coalesce(sum(c.cache_creation_tokens), 0) AS cache_creation_tokens,
                coalesce(sum(c.reasoning_tokens), 0) AS reasoning_tokens
         ORDER BY total_cost DESC",
        json!({ "pid": project_id }),
    ).await?;
    Ok(rows.iter().map(|r| {
        let user_id = r["user_id"].as_str().unwrap_or("").to_string();
        (user_id, row_to_summary(r))
    }).collect())
}

pub async fn deployment_llm_calls(neo4j: &Neo4jClient, deployment_id: &str, limit: usize) -> Result<Vec<Value>> {
    let rows = neo4j.query_read(
        "MATCH (d:Deployment {id: $did})-[:INCURRED]->(c:LlmCall)
         RETURN c.id AS id, c.scope AS scope, c.turn_id AS turn_id, c.model AS model, c.kind AS kind,
                c.input_tokens AS input_tokens, c.output_tokens AS output_tokens,
                c.cache_read_tokens AS cache_read_tokens, c.cache_creation_tokens AS cache_creation_tokens,
                c.reasoning_tokens AS reasoning_tokens,
                c.cost_microusd AS cost_microusd, c.duration_ms AS duration_ms,
                c.iteration_index AS iteration_index, c.succeeded AS succeeded, c.created_at AS created_at
         ORDER BY c.created_at DESC
         LIMIT $limit",
        json!({ "did": deployment_id, "limit": limit as i64 }),
    ).await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LlmProviderConfig;

    fn anthropic_config_with_pricing() -> LlmProviderConfig {
        toml::from_str(r#"
            provider = "anthropic"
            model = "claude-sonnet-4-6"
            api_key = "k"
            pricing = { input_per_1k = 3000, cache_read_per_1k = 300, cache_creation_per_1k = 3750, output_per_1k = 15000, reasoning_per_1k = 15000 }
        "#).unwrap()
    }

    fn gemini_config_no_pricing() -> LlmProviderConfig {
        toml::from_str(r#"
            provider = "gemini"
            model = "gemini-flash"
            api_key = "k"
        "#).unwrap()
    }

    #[test]
    fn cost_scope_str_roundtrip() {
        assert_eq!(CostScope::Chat.as_str(), "chat");
        assert_eq!(CostScope::Design.as_str(), "design");
        assert_eq!(CostScope::Provision.as_str(), "provision");
        assert_eq!(CostScope::Proposal.as_str(), "proposal");
        assert_eq!(CostScope::Title.as_str(), "title");
    }

    #[test]
    fn pricing_table_builds_from_configs() {
        let configs = vec![anthropic_config_with_pricing(), gemini_config_no_pricing()];
        let table = PricingTable::from_configs(&configs);
        assert!(table.lookup("anthropic", "claude-sonnet-4-6").is_some());
        assert!(table.lookup("gemini", "gemini-flash").is_some(), "gemini-flash should get default pricing");
    }

    #[test]
    fn pricing_table_prices_known_model() {
        let configs = vec![anthropic_config_with_pricing()];
        let table = PricingTable::from_configs(&configs);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 200, cache_creation_tokens: 100, reasoning_tokens: 50 };
        let cost = table.price_call(&usage, "anthropic", "claude-sonnet-4-6");
        assert!(cost > 0);
        let expected = (1000 * 3000 + 500 * 15000 + 200 * 300 + 100 * 3750 + 50 * 15000) / 1000;
        assert_eq!(cost, expected);
    }

    #[test]
    fn default_pricing_for_gemini_flash() {
        let configs = vec![gemini_config_no_pricing()];
        let table = PricingTable::from_configs(&configs);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let cost = table.price_call(&usage, "gemini", "gemini-flash");
        assert!(cost > 0, "gemini-flash should have default pricing");
    }

    #[test]
    fn default_pricing_for_unknown_model() {
        let cfg: LlmProviderConfig = toml::from_str(r#"
            provider = "openai-compatible"
            base_url = "https://example.com"
            model = "some-unknown-model"
            api_key = "k"
        "#).unwrap();
        let table = PricingTable::from_configs(&vec![cfg]);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let cost = table.price_call(&usage, "openai-compatible", "some-unknown-model");
        assert!(cost > 0, "unknown models should get fallback pricing");
    }

    #[test]
    fn default_pricing_for_kimi() {
        let cfg: LlmProviderConfig = toml::from_str(r#"
            provider = "openai-compatible"
            base_url = "https://openrouter.ai/api/v1"
            model = "moonshotai/kimi-k3"
            api_key = "k"
        "#).unwrap();
        let table = PricingTable::from_configs(&vec![cfg]);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let cost = table.price_call(&usage, "openai-compatible", "moonshotai/kimi-k3");
        assert!(cost > 0, "kimi-k3 should get default pricing");
    }

    #[test]
    fn pricing_table_unknown_model_gets_fallback_pricing() {
        let table = PricingTable::from_configs(&[]);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        assert!(table.price_call(&usage, "unknown", "unknown-model") > 0);
    }

    #[test]
    fn build_record_computes_cost_from_pricing() {
        let configs = vec![anthropic_config_with_pricing()];
        let table = PricingTable::from_configs(&configs);
        let used = UsedProvider { provider_id: "anthropic-0".into(), kind: "anthropic".into(), model: "claude-sonnet-4-6".into() };
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let record = build_record(
            CostScope::Chat, "turn-1", "user-1", &used, usage.clone(), &table, 1200, Some(0), true,
            Some("proj-1"), Some("conv-1"), None, None, None,
        );
        assert_eq!(record.cost_microusd, (1000 * 3000 + 500 * 15000) / 1000);
        assert_eq!(record.scope, CostScope::Chat);
        assert_eq!(record.project_id.as_deref(), Some("proj-1"));
        assert_eq!(record.conversation_id.as_deref(), Some("conv-1"));
        assert!(record.succeeded);
    }

    #[test]
    fn build_record_unknown_model_gets_fallback_cost() {
        let table = PricingTable::from_configs(&[]);
        let used = UsedProvider { provider_id: "x".into(), kind: "unknown".into(), model: "m".into() };
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let record = build_record(
            CostScope::Design, "turn-1", "user-1", &used, usage, &table, 100, None, true,
            None, None, Some("dep-1"), None, Some("art-1"),
        );
        assert!(record.cost_microusd > 0, "should get fallback pricing");
        assert_eq!(record.deployment_id.as_deref(), Some("dep-1"));
        assert_eq!(record.artifact_id.as_deref(), Some("art-1"));
    }

    #[test]
    fn cost_summary_add_accumulates() {
        let mut a = CostSummary { total_microusd: 100, total_calls: 2, total_input_tokens: 10, total_output_tokens: 5, total_cache_read_tokens: 0, total_cache_creation_tokens: 0, total_reasoning_tokens: 0 };
        let b = CostSummary { total_microusd: 200, total_calls: 3, total_input_tokens: 20, total_output_tokens: 10, total_cache_read_tokens: 0, total_cache_creation_tokens: 0, total_reasoning_tokens: 0 };
        a.add(&b);
        assert_eq!(a.total_microusd, 300);
        assert_eq!(a.total_calls, 5);
        assert_eq!(a.total_input_tokens, 30);
    }

    #[test]
    fn cost_summary_from_usage_and_cost() {
        let usage = Usage { input_tokens: 100, output_tokens: 50, cache_read_tokens: 10, cache_creation_tokens: 5, reasoning_tokens: 2 };
        let s = CostSummary::from_usage_and_cost(&usage, 42100, 3);
        assert_eq!(s.total_microusd, 42100);
        assert_eq!(s.total_calls, 3);
        assert_eq!(s.total_input_tokens, 100);
        assert_eq!(s.total_cache_read_tokens, 10);
    }

    #[test]
    fn cost_summary_formatted_cost() {
        let s = CostSummary { total_microusd: 42100, ..Default::default() };
        assert_eq!(s.formatted_cost(), "$0.042100");
    }

    #[test]
    fn pricing_config_parses_from_toml() {
        let cfg = anthropic_config_with_pricing();
        let pc = cfg.pricing().expect("pricing should be present");
        assert_eq!(pc.input_per_1k, 3000);
        assert_eq!(pc.output_per_1k, 15000);
        assert_eq!(pc.cache_read_per_1k, 300);
        assert_eq!(pc.cache_creation_per_1k, 3750);
    }

    #[test]
    fn pricing_config_absent_when_not_specified() {
        let cfg = gemini_config_no_pricing();
        assert!(cfg.pricing().is_none());
    }
}
