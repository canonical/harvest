use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

use crate::{
    agent::{Agent, AgentEvent},
    api::ProjectAgentBuilder,
    llm::{
        types::{LlmResponse, Message},
        LlmProvider,
    },
    neo4j::Neo4jClient,
    overview,
};

const ENV_DOC_USER_PREFIX: &str =
    "Based on the following project conversations, write a comprehensive technical document describing:\n\
     - Services and infrastructure mentioned\n\
     - What the team has been working on\n\
     - Configuration and deployment details\n\
     - Current state of systems\n\
     - Any issues or important context\n\n\
     Project conversations:\n\n";

const STATUS_USER_PREFIX: &str =
    "Generate a modern status dashboard as an HTML snippet for this project's environment.\n\
     \n\
     STRICT FORMAT RULES — output ONLY what is described here, nothing else:\n\
     • Start with a single <style> block, then the HTML markup. No <!DOCTYPE>, <html>, <head>, or <body> tags.\n\
     • No JavaScript, no external resources, no inline event handlers (on*).\n\
     • Prefix every CSS class with \"ov-\" to avoid collisions with the parent page.\n\
     • Use ONLY these pre-defined CSS variables (they handle light/dark automatically):\n\
       --bg-surface  --bg-surface-alt  --bg-page\n\
       --text-primary  --text-secondary  --text-muted\n\
       --border-color  --border-subtle\n\
     • Define your own status-color variables inside :root:\n\
       --ov-green:#0e8420; --ov-yellow:#b68a00; --ov-red:#c7162b; --ov-grey:#888888;\n\
       and add a @media(prefers-color-scheme:dark) block with brighter variants.\n\
     \n\
     LAYOUT:\n\
     • Immediately after the <style> block, include ONE <p class=\"ov-desc\"> element: 1–2 sentences\n\
       summarising what this environment is (its purpose, stack, or key services). Style it:\n\
       font-size 0.875rem, color var(--text-secondary), margin 0 0 1rem 0, line-height 1.5.\n\
     • Then the outer wrapper: CSS grid, auto-fit columns min 220px, gap 1rem.\n\
     • Each section = a card: border 1px solid var(--border-color), border-radius 8px,\n\
       background var(--bg-surface), padding 1rem 1.25rem.\n\
     • Card heading: font-size 0.6875rem, uppercase, font-weight 700, letter-spacing 0.07em,\n\
       color var(--text-muted), padding-bottom 0.375rem, margin-bottom 0.625rem,\n\
       border-bottom 1px solid var(--border-subtle).\n\
     • Each row: display flex, justify-content space-between, font-size 0.875rem,\n\
       padding 0.25rem 0, border-bottom 1px solid var(--border-subtle) (omit on last row).\n\
     • Status dot: 8px circle, inline-block, margin-right 0.4rem, vertical-align middle.\n\
     • Row label: color var(--text-secondary). Row value: color var(--text-primary), font-weight 500.\n\
     • At most 3 cards, at most 6 rows each. Total output ≤ 300 words.\n\
     • Output raw HTML — do NOT wrap in a markdown code fence (no ``` delimiters of any kind).\n\
     \n\
     Environment context:\n\n";

const MAX_DIRECT_CHARS: usize = 8_000;

pub async fn run(
    llm:           Arc<dyn LlmProvider>,
    agent_builder: Arc<ProjectAgentBuilder>,
    neo4j:         Arc<Neo4jClient>,
    project_id:    &str,
    events_tx:     broadcast::Sender<String>,
) -> Result<()> {
    let _ = events_tx.send(stage_event("Analyzing conversations"));

    let conv_text = overview::all_conversations_text(&neo4j, project_id).await?;
    if conv_text.trim().is_empty() {
        tracing::debug!(project_id, "overview pipeline: no conversations, skipping");
        return Ok(());
    }

    let env_doc = if conv_text.len() > MAX_DIRECT_CHARS {
        let _ = events_tx.send(stage_event("Building environment model"));
        generate_env_doc(&*llm, &conv_text).await?
    } else {
        conv_text
    };

    let _ = events_tx.send(stage_event("Querying agents"));
    let agent  = agent_builder.build(project_id.to_string());
    let status = generate_status(&agent, &env_doc, &events_tx).await?;

    overview::save(&neo4j, project_id, &env_doc, &status).await?;
    tracing::info!(project_id, "overview pipeline: completed");
    Ok(())
}

async fn generate_env_doc(llm: &dyn LlmProvider, conversations: &str) -> Result<String> {
    let user_text = format!("{ENV_DOC_USER_PREFIX}{conversations}");
    let messages  = vec![Message::user(user_text)];
    let resp      = llm.chat(&messages, &[]).await?;
    Ok(match resp {
        LlmResponse::Message { text } => text,
        LlmResponse::ToolCalls(_)     => String::new(),
    })
}

async fn generate_status(
    agent:     &Arc<Agent>,
    env_doc:   &str,
    events_tx: &broadcast::Sender<String>,
) -> Result<String> {
    let query        = format!("{STATUS_USER_PREFIX}{env_doc}");
    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
    let agent_clone  = Arc::clone(agent);
    let query_clone  = query.clone();

    tokio::spawn(async move {
        agent_clone.query_streaming(&query_clone, &[], &[], agent_tx).await;
    });

    let mut answer = String::new();
    while let Some(ev) = agent_rx.recv().await {
        match &ev {
            AgentEvent::ToolCall { name, .. } => {
                let _ = events_tx.send(tool_event(&format_tool_name(name)));
            }
            AgentEvent::Done { answer: a, .. } => {
                answer = a.clone();
            }
            _ => {}
        }
    }
    Ok(strip_code_fence(answer))
}

fn strip_code_fence(s: String) -> String {
    let trimmed = s.trim();
    let Some(after_ticks) = trimmed.strip_prefix("```") else {
        return s;
    };
    let content_start = after_ticks.find('\n').map(|i| i + 1).unwrap_or(0);
    let content = &after_ticks[content_start..];
    if let Some(inner) = content.trim_end().strip_suffix("```") {
        inner.trim().to_string()
    } else {
        s
    }
}

fn stage_event(action: &str) -> String {
    serde_json::json!({ "type": "overview_step", "kind": "stage", "action": action }).to_string()
}

fn tool_event(action: &str) -> String {
    serde_json::json!({ "type": "overview_step", "kind": "tool", "action": action }).to_string()
}

fn format_tool_name(name: &str) -> String {
    match name {
        "list_agents"   => "Listing agents".into(),
        "run_command"   => "Running command".into(),
        "get_symbols"   => "Fetching symbols".into(),
        "search_code"   => "Searching code".into(),
        _               => name.replace('_', " "),
    }
}
