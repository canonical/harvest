use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::api::{AgentEvent, IntentMode, QueryRequest, Source, UsedProvider};
use crate::app::AppData;
use crate::widgets::markdown::render_markdown;

use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct StepEntry {
    name: String,
    input: serde_json::Value,
    #[allow(dead_code)]
    input_preview: String,
    result_preview: Option<String>,
    open: bool,
}

#[derive(Clone, Debug)]
enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
struct MessageBlock {
    role: Role,
    timestamp: String,
    text: String,
    steps: Vec<StepEntry>,
    sources: Vec<Source>,
    intent: Option<IntentMode>,
    phase: Option<String>,
    thinking: String,
    streaming: bool,
    question: Option<(String, Vec<String>)>,
    confirm: Option<ConfirmCard>,
    provider: Option<UsedProvider>,
    parallel_research: Vec<ParallelLead>,
    duration_ms: u64,
}

#[derive(Clone, Debug)]
struct ConfirmCard {
    id: String,
    name: String,
    description: String,
}

#[derive(Clone, Debug)]
struct ParallelLead {
    name: String,
    status: String,
    preview: String,
}

pub struct ChatView {
    messages: Vec<MessageBlock>,
    input: String,
    scroll: usize,
    cur_steps: Vec<StepEntry>,
    cur_thinking: String,
    cur_intent: Option<IntentMode>,
    cur_phase: Option<String>,
    cur_sources: Vec<Source>,
    cur_provider: Option<UsedProvider>,
    streaming: bool,
    citation_open: Option<(Source, Option<crate::api::SymbolSource>)>,
    error_banner: Option<String>,
    history: Vec<String>,
    history_idx: Option<usize>,
    conversation_id: Option<String>,
    show_history: bool,
    history_state: ListState,
    step_selected: Option<usize>,
    step_mode: bool,
    #[allow(dead_code)]
    source_idx: Option<usize>,
}

const CHAIN_BORDER: char = '│';
const CHAIN_INDENT: &str = " │  ";
const SECTION_GAP: &str = "";

fn intent_label(mode: &IntentMode) -> &'static str {
    match mode {
        IntentMode::Research => "Researching",
        IntentMode::Action => "Executing",
        IntentMode::Hybrid => "Researching + Executing",
        IntentMode::Conversational => "Answering",
    }
}

fn intent_color(mode: &IntentMode) -> Color {
    match mode {
        IntentMode::Research => Color::Blue,
        IntentMode::Action => Color::Red,
        IntentMode::Hybrid => Color::Blue,
        IntentMode::Conversational => Color::DarkGray,
    }
}

fn intent_short(mode: &IntentMode) -> &'static str {
    match mode {
        IntentMode::Research => "research",
        IntentMode::Action => "action",
        IntentMode::Hybrid => "hybrid",
        IntentMode::Conversational => "conversational",
    }
}

fn label_span(text: &str) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
}

fn chain_line(spans: Vec<Span<'_>>) -> Line<'_> {
    let mut out = vec![Span::styled(CHAIN_INDENT.to_string(), Style::default().fg(Color::DarkGray))];
    out.extend(spans);
    Line::from(out)
}

fn chain_header(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(" │ ".to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(
            text.to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn chain_close(color: Color) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {}", CHAIN_BORDER),
        Style::default().fg(color),
    ))
}

fn group_sources(sources: &[Source]) -> Vec<(String, String, String, Vec<&Source>)> {
    let mut groups: BTreeMap<(String, String, String), Vec<&Source>> = BTreeMap::new();
    for s in sources {
        let key = (s.repo.clone(), s.version.clone(), s.file.clone());
        groups.entry(key).or_default().push(s);
    }
    groups
        .into_iter()
        .map(|((repo, version, file), srcs)| (repo, version, file, srcs))
        .collect()
}

fn describe_tool_call(name: &str, input: &serde_json::Value) -> String {
    let get = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let get_raw = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "list_repositories" => "Discovering available repositories".to_string(),
        "list_agents" => "Checking connected agents".to_string(),
        "list_skills" => "Listing available skills".to_string(),
        "search_symbols" => {
            let q = get("query");
            let kind = {
                let k = get("kind");
                if k.is_empty() || k == "any" { String::new() } else { format!(" {}s", k) }
            };
            let scope = {
                let r = get("repo");
                if r.is_empty() { String::new() } else { format!(" in {}", r) }
            };
            format!("Searching for \"{}\"{}{}", q, kind, scope)
        }
        "get_symbol_source" => format!("Reading source of {}", get("name")),
        "get_file_symbols" => {
            let f = get("file");
            let short = f.rsplit('/').next().unwrap_or(f);
            format!("Scanning symbols in {}", short)
        }
        "find_callers" => format!("Tracing callers of {}", get("function_name")),
        "find_callees" => format!("Tracing calls made by {}", get("function_name")),
        "get_imports" => {
            let f = get("file");
            let short = f.rsplit('/').next().unwrap_or(f);
            format!("Checking imports in {}", short)
        }
        "compare_symbol_across_versions" => {
            let sym = get("name");
            let va = get("version_a");
            let vb = get("version_b");
            if !va.is_empty() && !vb.is_empty() {
                format!("Comparing {} {} → {}", sym, va, vb)
            } else {
                format!("Comparing {} across versions", sym)
            }
        }
        "run_cypher" => "Querying the code graph".to_string(),
        "run_command" => {
            let cmd = get_raw("command");
            let agent = get("agent_id");
            let host = get("hostname");
            let target = if !host.is_empty() { host } else if !agent.is_empty() { agent } else { "agent" };
            let cmd_short = if cmd.len() > 28 { format!("{}…", &cmd[..27]) } else { cmd.to_string() };
            format!("Running \"{}\" on {}", cmd_short, target)
        }
        "create_lxd_agent" => format!("Provisioning agent {}", get("name")),
        "delete_agent" => format!("Deleting agent {}", get("agent_id")),
        "list_port_forwards" => "Listing port forwards".to_string(),
        "create_port_forward" => {
            let lp = input.get("local_port").and_then(|v| v.as_i64()).unwrap_or(0);
            let rp = input.get("remote_port").and_then(|v| v.as_i64()).unwrap_or(0);
            format!("Creating port forward {} → {}", lp, rp)
        }
        "update_port_forward" => format!("Updating port forward {}", get("id")),
        "delete_port_forward" => format!("Deleting port forward {}", get("id")),
        "generate_artifact" => format!("Generating artifact {}", get("name")),
        "read_provision_bundle" => "Reading provision bundle".to_string(),
        "set_execution_plan" => "Setting execution plan".to_string(),
        "update_product_template" => "Updating product template".to_string(),
        "link_deployment_artifact" => "Linking deployment artifact".to_string(),
        "run_terraform_plan" => "Running terraform plan".to_string(),
        "run_terraform_apply" => "Running terraform apply".to_string(),
        "run_terraform_destroy" => "Running terraform destroy".to_string(),
        "load_skill" => format!("Loading skill {}", get("name")),
        _ => name.replace('_', " "),
    }
}

fn summarize_tool_result(name: &str, preview: &str, input: &serde_json::Value) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(preview) {
        Ok(v) => v,
        Err(_) => return truncate(preview, 60),
    };

    let get = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("");

    match name {
        "list_repositories" => {
            if let Some(arr) = parsed.as_array() {
                return format!("{} repositories found", arr.len());
            }
            truncate(preview, 60)
        }
        "search_symbols" => {
            if let Some(arr) = parsed.as_array() {
                let n = arr.len();
                let names: Vec<&str> = arr.iter().take(3).filter_map(|v| v.get("name").and_then(|n| n.as_str())).collect();
                if n == 0 {
                    return "no matches".to_string();
                }
                let label = if n <= 3 { names.join(", ") } else { format!("{}… +{} more", names.join(", "), n - 3) };
                return format!("{} results: {}", n, label);
            }
            truncate(preview, 60)
        }
        "get_file_symbols" => {
            if let Some(arr) = parsed.as_array() {
                return format!("{} symbols found", arr.len());
            }
            truncate(preview, 60)
        }
        "find_callers" | "find_callees" => {
            if let Some(arr) = parsed.as_array() {
                let n = arr.len();
                if n == 0 {
                    return "none found".to_string();
                }
                let names: Vec<&str> = arr.iter().take(2).filter_map(|v| v.get("name").and_then(|n| n.as_str())).collect();
                return format!("{}: {}", n, names.join(", "));
            }
            truncate(preview, 60)
        }
        "get_imports" => {
            if let Some(arr) = parsed.as_array() {
                return format!("{} imports", arr.len());
            }
            truncate(preview, 60)
        }
        "get_symbol_source" => {
            if let Some(obj) = parsed.as_array().and_then(|a| a.first()) {
                let n = obj.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let start = obj.get("start_line").and_then(|v| v.as_i64()).unwrap_or(0);
                let end = obj.get("end_line").and_then(|v| v.as_i64()).unwrap_or(0);
                return format!("{} L{}–{}", n, start, end);
            }
            truncate(preview, 60)
        }
        "run_cypher" => {
            if let Some(arr) = parsed.as_array() {
                if arr.is_empty() {
                    return "0 rows".to_string();
                }
                let keys: Vec<&str> = arr.first()
                    .and_then(|v| v.as_object())
                    .map(|o| o.keys().take(3).map(|k| k.as_str()).collect())
                    .unwrap_or_default();
                return format!("{} rows: {}", arr.len(), keys.join(", "));
            }
            truncate(preview, 60)
        }
        "run_command" => {
            let stdout = parsed.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
            let exit = parsed.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let first_line = stdout.lines().next().unwrap_or("");
            if !first_line.is_empty() {
                return format!("[exit {}] {}", exit, truncate(first_line, 40));
            }
            format!("[exit {}]", exit)
        }
        "list_agents" => {
            if let Some(arr) = parsed.as_array() {
                let online = arr.iter().filter(|a| a.get("online").and_then(|v| v.as_bool()).unwrap_or(false)).count();
                return format!("{} agents ({} online)", arr.len(), online);
            }
            truncate(preview, 60)
        }
        "list_skills" => {
            if let Some(arr) = parsed.as_array() {
                return format!("{} skills", arr.len());
            }
            truncate(preview, 60)
        }
        "list_port_forwards" => {
            if let Some(arr) = parsed.as_array() {
                return format!("{} port forwards", arr.len());
            }
            truncate(preview, 60)
        }
        "load_skill" => {
            let skill_name = get("name");
            if !skill_name.is_empty() {
                return format!("loaded: {}", skill_name);
            }
            truncate(preview, 60)
        }
        "compare_symbol_across_versions" => {
            if let Some(arr) = parsed.as_array() {
                return format!("{} versions compared", arr.len());
            }
            truncate(preview, 60)
        }
        _ => {
            if let Some(arr) = parsed.as_array() {
                if arr.is_empty() {
                    return "empty".to_string();
                }
                return format!("{} items", arr.len());
            }
            if let Some(obj) = parsed.as_object() {
                if obj.is_empty() {
                    return "empty".to_string();
                }
                let keys: Vec<&str> = obj.keys().take(3).map(|k| k.as_str()).collect();
                return format!("{{{}{}}}", keys.join(", "), if obj.len() > 3 { ", …" } else { "" });
            }
            truncate(preview, 60)
        }
    }
}

fn extract_result_lines(name: &str, preview: &str, _input: &serde_json::Value, width: usize) -> Vec<String> {
    let parsed: serde_json::Value = match serde_json::from_str(preview) {
        Ok(v) => v,
        Err(_) => {
            return preview.lines().take(8).map(|l| l.to_string()).collect();
        }
    };

    let w = width.saturating_sub(8).max(20);

    match name {
        "run_command" => {
            let stdout = parsed.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
            let stderr = parsed.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
            let exit = parsed.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let mut out = Vec::new();
            for l in stdout.lines().take(w.min(10)) {
                out.push(format!("  {}", truncate(l, w)));
            }
            if !stderr.is_empty() {
                out.push("  [stderr]".to_string());
                for l in stderr.lines().take(3) {
                    out.push(format!("  {}", truncate(l, w)));
                }
            }
            out.push(format!("  [exit {}]", exit));
            out
        }
        "get_symbol_source" | "get_file_symbols" => {
            if let Some(arr) = parsed.as_array() {
                let mut out = Vec::new();
                for item in arr.iter().take(3) {
                    let n = item.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let start = item.get("start_line").and_then(|v| v.as_i64()).unwrap_or(0);
                    let end = item.get("end_line").and_then(|v| v.as_i64()).unwrap_or(0);
                    out.push(format!("  {} [{}] L{}–{}", n, kind, start, end));
                }
                if arr.len() > 3 {
                    out.push(format!("  …+{} more", arr.len() - 3));
                }
                out
            } else {
                vec![truncate(preview, w)]
            }
        }
        "search_symbols" | "find_callers" | "find_callees" => {
            if let Some(arr) = parsed.as_array() {
                let mut out = Vec::new();
                for item in arr.iter().take(5) {
                    let n = item.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let f = item.get("file").and_then(|v| v.as_str()).unwrap_or("");
                    let short = f.rsplit('/').next().unwrap_or(f);
                    let line = item.get("start_line").and_then(|v| v.as_i64()).unwrap_or(0);
                    out.push(format!("  {} — {}:{}", truncate(n, 30), short, line));
                }
                if arr.len() > 5 {
                    out.push(format!("  …+{} more", arr.len() - 5));
                }
                out
            } else {
                vec![truncate(preview, w)]
            }
        }
        "list_repositories" | "list_agents" | "list_skills" | "list_port_forwards" => {
            if let Some(arr) = parsed.as_array() {
                let name_key = if name == "list_repositories" { "name" } else if name == "list_agents" { "hostname" } else { "name" };
                let mut out = Vec::new();
                for item in arr.iter().take(5) {
                    let n = item.get(name_key).and_then(|v| v.as_str()).unwrap_or("?");
                    if name == "list_agents" {
                        let online = item.get("online").and_then(|v| v.as_bool()).unwrap_or(false);
                        let dot = if online { "●" } else { "○" };
                        out.push(format!("  {} {}", dot, n));
                    } else {
                        out.push(format!("  • {}", n));
                    }
                }
                if arr.len() > 5 {
                    out.push(format!("  …+{} more", arr.len() - 5));
                }
                out
            } else {
                vec![truncate(preview, w)]
            }
        }
        _ => {
            if let Some(arr) = parsed.as_array() {
                let mut out = Vec::new();
                for item in arr.iter().take(5) {
                    let s = serde_json::to_string(item).unwrap_or_default();
                    out.push(format!("  {}", truncate(&s, w)));
                }
                if arr.len() > 5 {
                    out.push(format!("  …+{} more", arr.len() - 5));
                }
                out
            } else {
                preview.lines().take(8).map(|l| format!("  {}", truncate(l, w))).collect()
            }
        }
    }
}

impl ChatView {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            scroll: 0,
            cur_steps: Vec::new(),
            cur_thinking: String::new(),
            cur_intent: None,
            cur_phase: None,
            cur_sources: Vec::new(),
            cur_provider: None,
            streaming: false,
            citation_open: None,
            error_banner: None,
            history: Vec::new(),
            history_idx: None,
            conversation_id: None,
            show_history: false,
            history_state: ListState::default(),
            step_selected: None,
            step_mode: false,
            source_idx: None,
        }
    }

    fn current_assistant(&mut self) -> Option<&mut MessageBlock> {
        self.messages
            .iter_mut()
            .rev()
            .find(|m| matches!(m.role, Role::Assistant) && m.streaming)
    }

    pub fn on_agent_event(&mut self, app: &mut AppData, ev: AgentEvent) {
        match ev {
            AgentEvent::Intent { mode } => {
                self.cur_intent = Some(mode.clone());
                if let Some(m) = self.current_assistant() {
                    m.intent = Some(mode);
                }
            }
            AgentEvent::Phase { label } => {
                self.cur_phase = Some(label.clone());
                if let Some(m) = self.current_assistant() {
                    m.phase = Some(label);
                }
            }
            AgentEvent::Thinking { text } | AgentEvent::ThinkingDelta { text } => {
                self.cur_thinking.push_str(&text);
                if let Some(m) = self.current_assistant() {
                    m.thinking.push_str(&text);
                }
            }
            AgentEvent::TextDelta { text } => {
                if let Some(m) = self.current_assistant() {
                    m.text.push_str(&text);
                }
            }
            AgentEvent::ToolCall { name, input } => {
                let input_preview = serde_json::to_string(&input)
                    .unwrap_or_default()
                    .chars()
                    .take(80)
                    .collect();
                let step = StepEntry {
                    name,
                    input: input.clone(),
                    input_preview,
                    result_preview: None,
                    open: false,
                };
                self.cur_steps.push(step.clone());
                if let Some(m) = self.current_assistant() {
                    m.steps.push(step);
                }
            }
            AgentEvent::ToolResult { name: _, preview } => {
                if let Some(step) = self.cur_steps.last_mut() {
                    step.result_preview = Some(preview.clone());
                }
                if let Some(m) = self.current_assistant() {
                    if let Some(step) = m.steps.last_mut() {
                        step.result_preview = Some(preview);
                    }
                }
            }
            AgentEvent::Question { question, choices } => {
                if let Some(m) = self.current_assistant() {
                    m.question = Some((question, choices));
                }
            }
            AgentEvent::ConfirmAction {
                id,
                name,
                description,
                ..
            } => {
                if let Some(m) = self.current_assistant() {
                    m.confirm = Some(ConfirmCard {
                        id,
                        name,
                        description,
                    });
                }
            }
            AgentEvent::Done {
                answer,
                sources,
                provider_used,
                duration_ms,
                ..
            } => {
                if let Some(m) = self.current_assistant() {
                    m.streaming = false;
                    m.text = answer;
                    m.sources = sources.clone();
                    m.provider = provider_used.clone();
                    m.duration_ms = duration_ms;
                }
                self.cur_sources = sources;
                self.cur_provider = provider_used;
                self.cur_steps.clear();
                self.cur_thinking.clear();
                self.cur_intent = None;
                self.cur_phase = None;
                self.streaming = false;
                self.step_mode = false;
                self.step_selected = None;
                app.status = "done".to_string();
            }
            AgentEvent::Error { message } => {
                self.error_banner = Some(message.clone());
                if let Some(m) = self.current_assistant() {
                    m.streaming = false;
                }
                self.streaming = false;
                app.status = format!("error: {message}");
            }
            AgentEvent::TitleUpdated { .. } => {}
            AgentEvent::ParallelResearchStarted { leads } => {
                if let Some(m) = self.current_assistant() {
                    m.parallel_research = leads
                        .iter()
                        .map(|l| ParallelLead {
                            name: l.clone(),
                            status: "running".to_string(),
                            preview: String::new(),
                        })
                        .collect();
                }
            }
            AgentEvent::ParallelResearchLeadDone {
                index,
                preview,
                ..
            } => {
                if let Some(m) = self.current_assistant() {
                    if let Some(lead) = m.parallel_research.get_mut(index) {
                        lead.status = "done".to_string();
                        lead.preview = preview;
                    }
                }
            }
            AgentEvent::ParallelResearchMergeStarted { .. } => {
                if let Some(m) = self.current_assistant() {
                    for lead in &mut m.parallel_research {
                        if lead.status == "running" {
                            lead.status = "done".to_string();
                        }
                    }
                }
            }
        }
    }

    pub fn on_chat_error(&mut self, app: &mut AppData, msg: String) {
        self.error_banner = Some(msg.clone());
        self.streaming = false;
        app.status = format!("error: {msg}");
    }

    fn send_query(&mut self, app: &mut AppData) {
        let q = self.input.trim().to_string();
        if q.is_empty() || self.streaming {
            return;
        }
        self.history.push(q.clone());
        self.history_idx = None;
        let now = chrono::Local::now().format("%H:%M").to_string();
        self.messages.push(MessageBlock {
            role: Role::User,
            timestamp: now,
            text: q.clone(),
            steps: Vec::new(),
            sources: Vec::new(),
            intent: None,
            phase: None,
            thinking: String::new(),
            streaming: false,
            question: None,
            confirm: None,
            provider: None,
            parallel_research: Vec::new(),
            duration_ms: 0,
        });
        let now2 = chrono::Local::now().format("%H:%M").to_string();
        self.messages.push(MessageBlock {
            role: Role::Assistant,
            timestamp: now2,
            text: String::new(),
            steps: Vec::new(),
            sources: Vec::new(),
            intent: None,
            phase: None,
            thinking: String::new(),
            streaming: true,
            question: None,
            confirm: None,
            provider: None,
            parallel_research: Vec::new(),
            duration_ms: 0,
        });
        self.input.clear();
        self.streaming = true;
        self.error_banner = None;
        self.step_mode = false;
        self.step_selected = None;
        app.status = "streaming…".to_string();

        let body = QueryRequest {
            query: q,
            conversation_id: self.conversation_id.clone(),
            provider_id: app.model_pick.as_ref().map(|(id, _)| id.clone()),
            model: app.model_pick.as_ref().map(|(_, m)| m.clone()),
        };
        app.spawn_stream(body);
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0), Constraint::Length(3)])
            .split(area);

        let header_line = if self.streaming {
            let phase = self.cur_phase.as_deref().unwrap_or("Thinking…");
            let intent_str = self
                .cur_intent
                .as_ref()
                .map(|m| intent_short(m))
                .unwrap_or("conversational");
            format!(" Chat  [{intent_str} · {phase}]")
        } else {
            " Chat".to_string()
        };
        let header_para = Paragraph::new(Line::from(vec![
            Span::styled(header_line, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
        ]))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header_para, chunks[0]);

        let transcript_area = chunks[1];
        let mut lines: Vec<Line> = Vec::new();

        if let Some(err) = &self.error_banner {
            lines.push(Line::from(vec![
                Span::styled(" ✖ ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(err.clone(), Style::default().fg(Color::Red)),
            ]));
            lines.push(Line::from(SECTION_GAP));
        }

        for m in &self.messages {
            self.render_message(m, &mut lines, transcript_area.width as usize);
        }

        if self.messages.is_empty() && self.error_banner.is_none() {
            lines.push(Line::from(Span::styled(
                " Welcome to Harvest TUI.",
                Style::default().fg(Color::Reset).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                " Type a question below and press Alt-Enter to send.",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(SECTION_GAP));
            lines.push(Line::from(label_span("EXAMPLES")));
            lines.push(Line::from(Span::styled(
                "   How does the retry logic work?",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "   Show me the callers of with_backoff",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let count = lines.len();
        let visible = transcript_area.height as usize;
        let max_scroll = count.saturating_sub(visible);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        let start = if count > visible {
            count - visible - (max_scroll - self.scroll).min(count - visible)
        } else {
            0
        };
        let visible_lines: Vec<Line> = lines
            .into_iter()
            .skip(start)
            .take(visible)
            .collect();
        let transcript = Paragraph::new(visible_lines)
            .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(transcript, transcript_area);

        let mut scroll_state = ScrollbarState::new(max_scroll).position(self.scroll);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            transcript_area,
            &mut scroll_state,
        );

        let input_hint = if self.step_mode {
            " Steps mode: j/k select, Enter expand, Esc exit "
        } else {
            " Query (Alt+Enter to send, Ctrl-H history) "
        };
        let input_para = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(self.input.clone()),
            Span::styled(" ", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(input_hint),
        );
        f.render_widget(input_para, chunks[2]);

        if self.show_history {
            self.render_history_popup(f, area, app);
        }

        if let Some((source, sym)) = &self.citation_open {
            let pop = centered(area, 70, 60);
            let content = sym
                .as_ref()
                .and_then(|s| s.source.clone())
                .unwrap_or_else(|| "(no source available)".to_string());
            let lines: Vec<Line> = content
                .lines()
                .enumerate()
                .map(|(i, l)| {
                    Line::from(vec![
                        Span::styled(
                            format!("{:>4} ", i + 1),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(l.to_string(), Style::default().fg(Color::Green)),
                    ])
                })
                .collect();
            let para = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " {}:{}:{} ",
                        source.file, source.line, source.repo
                    )),
            );
            f.render_widget(para, pop);
        }
    }

    fn render_message(&self, m: &MessageBlock, lines: &mut Vec<Line>, width: usize) {
        match m.role {
            Role::User => self.render_user_message(m, lines, width),
            Role::Assistant => self.render_assistant_message(m, lines, width),
        }
        lines.push(Line::from(SECTION_GAP));
    }

    fn render_user_message(&self, m: &MessageBlock, lines: &mut Vec<Line>, _width: usize) {
        lines.push(Line::from(vec![
            Span::styled(" ● ", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
            Span::styled("You", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", m.timestamp), Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(""));
        let indent = "   ";
        for l in m.text.lines() {
            lines.push(Line::from(Span::styled(
                format!("{indent}{l}"),
                Style::default().fg(Color::Reset),
            )));
        }
    }

    fn render_assistant_message(&self, m: &MessageBlock, lines: &mut Vec<Line>, width: usize) {
        lines.push(Line::from(vec![
            Span::styled(" ● ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("Assistant", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", m.timestamp), Style::default().fg(Color::DarkGray)),
        ]));

        if let Some(p) = &m.provider {
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(
                    format!(" {} ", p.model),
                    Style::default().fg(Color::LightRed),
                ),
                Span::styled(
                    format!("· {} ", p.kind),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }

        if m.duration_ms > 0 {
            let secs = m.duration_ms / 1000;
            let label = if secs >= 60 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                format!("{}.{:01}s", secs, (m.duration_ms % 1000) / 100)
            };
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(
                    format!(" {} ", label),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }

        if let Some(intent) = &m.intent {
            let color = intent_color(intent);
            let label = intent_label(intent);
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(
                    format!(" {} ", label),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        if let Some(phase) = &m.phase {
            if m.streaming {
                lines.push(Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled(
                        format!(" {} ", phase),
                        Style::default().fg(Color::Blue),
                    ),
                ]));
            }
        }

        if m.streaming {
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled("▮", Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK)),
            ]));
        }

        lines.push(Line::from(""));

        let has_chain = !m.thinking.is_empty()
            || !m.steps.is_empty()
            || !m.parallel_research.is_empty()
            || m.confirm.is_some();

        if has_chain {
            self.render_activity_chain(m, lines, width);
            lines.push(Line::from(""));
        }

        if !m.text.is_empty() {
            let rendered = render_markdown(&m.text, width);
            for l in rendered {
                lines.push(l);
            }
        } else if m.streaming && !has_chain {
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled("Thinking", Style::default().fg(Color::DarkGray)),
                Span::styled("…", Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK)),
            ]));
        }

        if m.streaming && m.text.is_empty() && !m.thinking.is_empty() && m.steps.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled("▮", Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK)),
            ]));
        }

        if !m.sources.is_empty() {
            lines.push(Line::from(""));
            self.render_sources(m, lines);
        }

        if let Some((question, choices)) = &m.question {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled("? ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                Span::styled(question.clone(), Style::default().fg(Color::Reset).add_modifier(Modifier::BOLD)),
            ]));
            for (i, c) in choices.iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::styled("     ", Style::default()),
                    Span::styled(format!("{}. ", i + 1), Style::default().fg(Color::Magenta)),
                    Span::styled(c.clone(), Style::default().fg(Color::Reset)),
                ]));
            }
            lines.push(Line::from(Span::styled(
                "     (press 1-9 or type your own answer below)",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    fn render_activity_chain(&self, m: &MessageBlock, lines: &mut Vec<Line>, width: usize) {
        let chain_color = if m.streaming { Color::Blue } else { Color::DarkGray };

        if !m.thinking.is_empty() {
            lines.push(chain_header("THINKING"));
            let wrapped = wrap_dim(&m.thinking, width.saturating_sub(6));
            for l in wrapped.into_iter().take(6) {
                lines.push(chain_line(vec![
                    Span::styled(l, Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
                ]));
            }
            lines.push(chain_close(chain_color));
        }

        if !m.parallel_research.is_empty() {
            lines.push(chain_header("PARALLEL RESEARCH"));
            for lead in &m.parallel_research {
                let (dot, c) = if lead.status == "done" {
                    ("✓", Color::Green)
                } else {
                    ("◐", Color::Yellow)
                };
                lines.push(chain_line(vec![
                    Span::styled(format!("{dot} "), Style::default().fg(c)),
                    Span::styled(lead.name.clone(), Style::default().fg(Color::Reset)),
                    Span::styled(
                        format!("  ({})", lead.status),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            lines.push(chain_close(chain_color));
        }

        if !m.steps.is_empty() {
            self.render_steps(m, lines, width, chain_color);
        }

        if let Some(confirm) = &m.confirm {
            lines.push(chain_header("CONFIRM ACTION"));
            lines.push(chain_line(vec![
                Span::styled("! ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(confirm.name.clone(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            ]));
            let wrapped = wrap_dim(&confirm.description, width.saturating_sub(6));
            for l in &wrapped {
                lines.push(chain_line(vec![
                    Span::styled(l.clone(), Style::default().fg(Color::Reset)),
                ]));
            }
            lines.push(chain_line(vec![
                Span::styled("[y] approve   [n] deny", Style::default().fg(Color::Yellow)),
            ]));
            lines.push(chain_close(chain_color));
        }
    }

    fn render_steps(&self, m: &MessageBlock, lines: &mut Vec<Line>, width: usize, chain_color: Color) {
        let groups = group_tool_calls(&m.steps);
        let mut step_idx = 0usize;
        let total = m.steps.len();

        if total > 1 {
            lines.push(chain_header(&format!("STEPS ({})", total)));
        } else {
            lines.push(chain_header("STEPS"));
        }

        for group in &groups {
            if group.steps.len() >= 3 {
                lines.push(chain_line(vec![
                    Span::styled(
                        format!(" {} ", group.noun()),
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("× {}", group.steps.len()),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                for (i, step) in group.steps.iter().enumerate() {
                    let is_selected = self.step_mode && self.step_selected == Some(step_idx);
                    self.render_step_line(step, is_selected, lines, width, i + 1, group.steps.len());
                    step_idx += 1;
                }
            } else {
                for step in &group.steps {
                    let is_selected = self.step_mode && self.step_selected == Some(step_idx);
                    self.render_step_line(step, is_selected, lines, width, 0, 0);
                    step_idx += 1;
                }
            }
        }

        if self.step_mode {
            lines.push(chain_line(vec![
                Span::styled(
                    " j/k select  Enter expand  Esc exit",
                    Style::default().fg(Color::Blue),
                ),
            ]));
        }

        lines.push(chain_close(chain_color));
    }

    fn render_step_line(
        &self,
        step: &StepEntry,
        selected: bool,
        lines: &mut Vec<Line>,
        width: usize,
        group_num: usize,
        group_len: usize,
    ) {
        let (icon, status_color) = match &step.result_preview {
            Some(_) => ("✓", Color::Green),
            None => ("◐", Color::Yellow),
        };

        let label = describe_tool_call(&step.name, &step.input);

        let prefix = if group_len > 0 {
            format!("  {}. ", group_num)
        } else {
            "  ".to_string()
        };

        let sel_marker = if selected { "▶" } else { " " };

        let result_summary = if let Some(preview) = &step.result_preview {
            summarize_tool_result(&step.name, preview, &step.input)
        } else {
            "running…".to_string()
        };

        lines.push(chain_line(vec![
            Span::styled(format!("{}{}", sel_marker, prefix), Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{icon} "), Style::default().fg(status_color)),
            Span::styled(label, Style::default().fg(Color::Reset)),
        ]));

        let result_color = match &step.result_preview {
            Some(preview) => {
                if preview.contains("\"error\"") || preview.starts_with("error") {
                    Color::Red
                } else {
                    Color::DarkGray
                }
            }
            None => Color::Yellow,
        };
        lines.push(chain_line(vec![
            Span::styled(format!("    → {}", result_summary), Style::default().fg(result_color)),
        ]));

        if step.open {
            if let Some(preview) = &step.result_preview {
                let detail_lines = extract_result_lines(&step.name, preview, &step.input, width);
                for l in &detail_lines {
                    lines.push(chain_line(vec![
                        Span::styled(l.clone(), Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
            let input_str = serde_json::to_string_pretty(&step.input).unwrap_or_default();
            for l in input_str.lines().take(4) {
                lines.push(chain_line(vec![
                    Span::styled(format!("  {}", truncate(l, width.saturating_sub(8))), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
    }

    fn render_sources(&self, m: &MessageBlock, lines: &mut Vec<Line>) {
        let count = m.sources.len();
        lines.push(Line::from(vec![
            Span::styled("   ", Style::default()),
            label_span(&format!("SOURCES ({count})")),
        ]));

        let groups = group_sources(&m.sources);
        for (repo, version, file, srcs) in &groups {
            let locs: Vec<String> = srcs
                .iter()
                .map(|s| {
                    if s.line > 0 {
                        format!("L{}", s.line)
                    } else {
                        "file".to_string()
                    }
                })
                .collect();

            lines.push(Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(file.clone(), Style::default().fg(Color::Reset).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("  {} {}", repo, version),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("       ", Style::default()),
                Span::styled(locs.join("  "), Style::default().fg(Color::Blue)),
            ]));
        }
    }

    fn render_history_popup(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let pop = centered(area, 50, 60);
        let mut items: Vec<ListItem> = Vec::new();
        for c in &app.conversations {
            items.push(ListItem::new(vec![
                Line::from(Span::styled(
                    c.title.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!("  {} messages", c.message_count),
                    Style::default().fg(Color::DarkGray),
                )),
            ]));
        }
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                " no conversations",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" History (Esc close, Enter load, d delete) "),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, pop, &mut self.history_state);
    }

    pub async fn handle_key_async(&mut self, key: KeyEvent, app: &mut AppData) {
        if self.citation_open.is_some() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                self.citation_open = None;
            }
            return;
        }

        if self.show_history {
            self.handle_history_key(key, app).await;
            return;
        }

        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            return;
        }

        if key.code == KeyCode::Char('h') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.show_history = !self.show_history;
            if self.show_history {
                self.history_state.select(Some(0));
            }
            return;
        }

        if self.step_mode {
            self.handle_step_key(key, app).await;
            return;
        }

        if key.code == KeyCode::Tab && self.streaming {
            self.step_mode = true;
            self.step_selected = Some(0);
            return;
        }

        let is_send = (key.code == KeyCode::Enter
            && key.modifiers.contains(KeyModifiers::ALT))
            || (key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL));

        if is_send {
            self.send_query(app);
            return;
        }

        if let Some(m) = self.messages.iter_mut().rev().find(|m| m.streaming) {
            if let Some((_, choices)) = &m.question {
                if let KeyCode::Char(c) = key.code {
                    if let Some(n) = c.to_digit(10) {
                        if n > 0 && (n as usize) <= choices.len() {
                            self.input = choices[(n - 1) as usize].clone();
                            self.send_query(app);
                            return;
                        }
                    }
                }
            }
            if let Some(confirm) = &m.confirm {
                if key.code == KeyCode::Char('y') && key.modifiers == KeyModifiers::NONE {
                    let id = confirm.id.clone();
                    let pid = app.current_project_id();
                    let conv = self.conversation_id.clone();
                    let client = app.client.clone();
                    if let (Some(pid), Some(conv)) = (pid, conv) {
                        let body = serde_json::json!({
                            "results": [{
                                "action_id": id,
                                "approved": true,
                            }]
                        });
                        let _ = client
                            .post_text(
                                &format!("/projects/{pid}/conversations/{conv}/resume"),
                                &body,
                            )
                            .await;
                    }
                    if let Some(m) = self.messages.iter_mut().rev().find(|m| m.streaming) {
                        m.confirm = None;
                    }
                    return;
                }
                if key.code == KeyCode::Char('n') && key.modifiers == KeyModifiers::NONE {
                    let id = confirm.id.clone();
                    let pid = app.current_project_id();
                    let conv = self.conversation_id.clone();
                    let client = app.client.clone();
                    if let (Some(pid), Some(conv)) = (pid, conv) {
                        let body = serde_json::json!({
                            "results": [{
                                "action_id": id,
                                "approved": false,
                            }]
                        });
                        let _ = client
                            .post_text(
                                &format!("/projects/{pid}/conversations/{conv}/resume"),
                                &body,
                            )
                            .await;
                    }
                    if let Some(m) = self.messages.iter_mut().rev().find(|m| m.streaming) {
                        m.confirm = None;
                    }
                    return;
                }
            }
        }

        match key.code {
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                self.input.push(c);
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Up => {
                if !self.history.is_empty() {
                    let idx = self.history_idx.unwrap_or(self.history.len());
                    if idx > 0 {
                        let idx = idx - 1;
                        self.input = self.history[idx].clone();
                        self.history_idx = Some(idx);
                    }
                }
            }
            KeyCode::Down => {
                if let Some(idx) = self.history_idx {
                    if idx + 1 < self.history.len() {
                        let idx = idx + 1;
                        self.input = self.history[idx].clone();
                        self.history_idx = Some(idx);
                    } else {
                        self.input.clear();
                        self.history_idx = None;
                    }
                }
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(5);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(5);
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll = self.scroll.saturating_add(3);
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll = self.scroll.saturating_sub(3);
            }
            KeyCode::Enter => {
                if let Some(m) = self.messages.iter().rev().find(|m| m.streaming) {
                    if m.question.is_some() {
                        return;
                    }
                }
                self.send_query(app);
            }
            _ => {}
        }
    }

    async fn handle_history_key(&mut self, key: KeyEvent, app: &mut AppData) {
        match key.code {
            KeyCode::Esc => {
                self.show_history = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.history_state.selected().unwrap_or(0);
                self.history_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = app.conversations.len();
                if len > 0 {
                    let i = self.history_state.selected().unwrap_or(0);
                    self.history_state.select(Some((i + 1).min(len - 1)));
                }
            }
            KeyCode::Char('d') => {
                if let Some(idx) = self.history_state.selected() {
                    if let Some(c) = app.conversations.get(idx) {
                        let cid = c.id.clone();
                        if let Some(pid) = app.current_project_id() {
                            let client = app.client.clone();
                            let _ = client
                                .delete(&format!("/projects/{pid}/conversations/{cid}"))
                                .await;
                            app.refresh_side_data().await;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_step_key(&mut self, key: KeyEvent, _app: &mut AppData) {
        match key.code {
            KeyCode::Esc => {
                self.step_mode = false;
                self.step_selected = None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(m) = self.messages.iter().rev().find(|m| m.streaming) {
                    let len = m.steps.len();
                    if len > 0 {
                        let i = self.step_selected.unwrap_or(0);
                        self.step_selected = Some((i + 1).min(len - 1));
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.step_selected.unwrap_or(0);
                self.step_selected = Some(i.saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(idx) = self.step_selected {
                    if let Some(m) = self.messages.iter_mut().rev().find(|m| m.streaming) {
                        if let Some(step) = m.steps.get_mut(idx) {
                            step.open = !step.open;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

struct ToolGroup {
    steps: Vec<StepEntry>,
    name: String,
}

impl ToolGroup {
    fn noun(&self) -> String {
        match self.name.as_str() {
            "run_cypher" => "graph queries".to_string(),
            "get_symbol_source" => "source lookups".to_string(),
            "get_file_symbols" => "file scans".to_string(),
            "search_symbols" => "searches".to_string(),
            "find_callers" => "caller traces".to_string(),
            "find_callees" => "callee traces".to_string(),
            "get_imports" => "import checks".to_string(),
            "compare_symbol_across_versions" => "version comparisons".to_string(),
            _ => self.name.replace('_', " "),
        }
    }
}

fn group_tool_calls(steps: &[StepEntry]) -> Vec<ToolGroup> {
    let mut groups: Vec<ToolGroup> = Vec::new();
    for step in steps {
        if let Some(last) = groups.last_mut() {
            if last.name == step.name {
                last.steps.push(step.clone());
                continue;
            }
        }
        groups.push(ToolGroup {
            steps: vec![step.clone()],
            name: step.name.clone(),
        });
    }
    groups
}

fn wrap_dim(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.lines() {
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in raw.split_whitespace() {
            if cur.is_empty() {
                cur = word.to_string();
            } else if cur.len() + 1 + word.len() <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let h = area.height * height_pct / 100;
    let w = area.width * width_pct / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}