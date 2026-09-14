use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::api::{AgentEvent, IntentMode, QueryRequest, Source, UsedProvider};
use crate::app::AppData;
use crate::widgets::markdown::render_markdown;

#[derive(Clone, Debug)]
struct StepEntry {
    name: String,
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
    #[allow(dead_code)]
    source_idx: Option<usize>,
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
                ..
            } => {
                if let Some(m) = self.current_assistant() {
                    m.streaming = false;
                    m.text = answer;
                    m.sources = sources.clone();
                    m.provider = provider_used.clone();
                }
                self.cur_sources = sources;
                self.cur_provider = provider_used;
                self.cur_steps.clear();
                self.cur_thinking.clear();
                self.cur_intent = None;
                self.cur_phase = None;
                self.streaming = false;
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
        });
        self.input.clear();
        self.streaming = true;
        self.error_banner = None;
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

        let header = if self.streaming {
            let phase = self.cur_phase.as_deref().unwrap_or("thinking");
            let intent = match self.cur_intent {
                Some(IntentMode::Research) => "research",
                Some(IntentMode::Action) => "action",
                Some(IntentMode::Hybrid) => "hybrid",
                _ => "conversational",
            };
            format!(" Chat  [{intent} · {phase}] ▮ streaming")
        } else {
            " Chat".to_string()
        };
        let header_para = Paragraph::new(Line::from(Span::styled(
            header,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header_para, chunks[0]);

        let transcript_area = chunks[1];
        let mut lines: Vec<Line> = Vec::new();

        if let Some(err) = &self.error_banner {
            lines.push(Line::from(Span::styled(
                format!(" ⚠ {err}"),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
        }

        for m in &self.messages {
            let (marker, color) = match m.role {
                Role::User => ("▸ You", Color::Blue),
                Role::Assistant => ("◂ Assistant", Color::Green),
            };
            let mut header_spans: Vec<Span> = vec![
                Span::styled(
                    format!(" {marker} · {} ", m.timestamp),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(intent) = &m.intent {
                let label = match intent {
                    IntentMode::Research => "research",
                    IntentMode::Action => "action",
                    IntentMode::Hybrid => "hybrid",
                    IntentMode::Conversational => "conversational",
                };
                header_spans.push(Span::styled(
                    format!("[{label}] "),
                    Style::default().fg(Color::Magenta),
                ));
            }
            if let Some(phase) = &m.phase {
                header_spans.push(Span::styled(
                    format!("· {phase} "),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            if m.streaming {
                header_spans.push(Span::styled(
                    "▮",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
                ));
            }
            lines.push(Line::from(header_spans));
            lines.push(Line::from(""));

            if !m.thinking.is_empty() {
                lines.push(Line::from(Span::styled(
                    " thinking:",
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                )));
                for l in wrap_dim(&m.thinking, transcript_area.width as usize).into_iter().take(6) {
                    lines.push(Line::from(Span::styled(
                        format!("   {l}"),
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                    )));
                }
                lines.push(Line::from(""));
            }

            if !m.parallel_research.is_empty() {
                lines.push(Line::from(Span::styled(
                    " parallel research:",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )));
                for lead in &m.parallel_research {
                    let dot = if lead.status == "done" { "●" } else { "◐" };
                    let c = if lead.status == "done" {
                        Color::Green
                    } else {
                        Color::Yellow
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("   {dot} "), Style::default().fg(c)),
                        Span::styled(lead.name.clone(), Style::default().fg(Color::Reset)),
                        Span::styled(
                            format!(" ({})", lead.status),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                lines.push(Line::from(""));
            }

            if !m.text.is_empty() {
                let rendered = render_markdown(&m.text, transcript_area.width as usize);
                for l in rendered {
                    lines.push(l);
                }
            } else if m.streaming && m.steps.is_empty() && m.thinking.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  ▮",
                    Style::default().fg(Color::Yellow),
                )));
            }

            if !m.steps.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " steps:",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )));
                for step in m.steps.iter() {
                    let arrow = if step.open { "▼" } else { "▸" };
                    let result = step
                        .result_preview
                        .as_ref()
                        .map(|r| format!("→ {}", truncate(r, 40)))
                        .unwrap_or_else(|| "↻".to_string());
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("   {arrow} "),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            step.name.clone(),
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::styled(
                            format!("({})", truncate(&step.input_preview, 40)),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("  {result}"),
                            Style::default().fg(Color::Green),
                        ),
                    ]));
                }
                lines.push(Line::from(""));
            }

            if let Some((question, choices)) = &m.question {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(" ❓ {question}"),
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                )));
                for (i, c) in choices.iter().enumerate() {
                    lines.push(Line::from(Span::styled(
                        format!("   {}. {c}", i + 1),
                        Style::default().fg(Color::Reset),
                    )));
                }
                lines.push(Line::from(Span::styled(
                    "   (press 1-9 to answer or type below)",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
            }

            if let Some(confirm) = &m.confirm {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(" ⚠ confirm: {} — {}", confirm.name, confirm.description),
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    "   [y] approve   [n] deny",
                    Style::default().fg(Color::Yellow),
                )));
                lines.push(Line::from(""));
            }

            if !m.sources.is_empty() {
                lines.push(Line::from(Span::styled(
                    " Sources:",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )));
                for s in &m.sources {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "   [{}:{}:{}:{}]",
                            s.repo, s.version, s.file, s.line
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::UNDERLINED),
                    )));
                }
                lines.push(Line::from(""));
            }

            lines.push(Line::from(""));
        }

        if self.messages.is_empty() && self.error_banner.is_none() {
            lines.push(Line::from(Span::styled(
                " Welcome to Harvest TUI. Type a question below and press Alt-Enter to send.",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Examples:",
                Style::default().fg(Color::Cyan),
            )));
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

        let input_para = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(self.input.clone()),
            Span::styled(" ", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Query (Alt+Enter to send, Ctrl-H history) "),
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