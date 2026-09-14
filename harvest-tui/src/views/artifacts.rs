use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::api::ArtifactInfo;
use crate::app::AppData;
use crate::widgets::markdown::render_markdown;

#[derive(Clone, Copy, PartialEq)]
enum Popup {
    None,
    Create,
    DeleteConfirm,
    RunAgent,
    RunResult,
}

pub struct ArtifactsView {
    state: ListState,
    detail: Option<ArtifactInfo>,
    popup: Popup,
    create_title: String,
    create_kind: usize,
    create_content: String,
    create_field: usize,
    run_agent: usize,
    run_action: usize,
    run_output: Option<String>,
}

const KINDS: [&str; 5] = ["markdown", "terraform", "terragrunt", "bash", "pdf"];
const ACTIONS: [&str; 3] = ["plan", "apply", "destroy"];

impl ArtifactsView {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            state,
            detail: None,
            popup: Popup::None,
            create_title: String::new(),
            create_kind: 0,
            create_content: String::new(),
            create_field: 0,
            run_agent: 0,
            run_action: 0,
            run_output: None,
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let header = Paragraph::new(Line::from(Span::styled(
            format!(" Artifacts  [ {} ]", app.artifacts.len()),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Min(0)])
            .split(chunks[1]);

        let mut items: Vec<ListItem> = Vec::new();
        for a in &app.artifacts {
            let color = match a.kind.as_str() {
                "terraform" | "terragrunt" => Color::Magenta,
                "markdown" => Color::Blue,
                "bash" => Color::Green,
                _ => Color::Reset,
            };
            items.push(ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!(" {} ", a.kind), Style::default().fg(color)),
                    Span::styled(a.title.clone(), Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from(Span::styled(
                    format!("   {}", a.updated_at),
                    Style::default().fg(Color::DarkGray),
                )),
            ]));
        }
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                " no artifacts",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .title(" List (c create, d download, D delete, r run) "),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, body[0], &mut self.state);

        let detail = self
            .state
            .selected()
            .and_then(|i| app.artifacts.get(i))
            .cloned()
            .or_else(|| self.detail.clone());

        let lines: Vec<Line> = if let Some(a) = &detail {
            if let Some(content) = &a.content {
                let w = body[1].width as usize;
                let mut rendered = render_markdown(content, w);
                let mut meta = vec![
                    Line::from(vec![
                        Span::styled(" title: ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.title.clone(), Style::default().add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled(" kind: ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.kind.clone(), Style::default()),
                    ]),
                    Line::from(vec![
                        Span::styled(" updated: ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.updated_at.clone(), Style::default()),
                    ]),
                    Line::from(""),
                ];
                meta.append(&mut rendered);
                meta
            } else {
                vec![
                    Line::from(vec![
                        Span::styled(" title: ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.title.clone(), Style::default()),
                    ]),
                    Line::from(vec![
                        Span::styled(" kind: ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.kind.clone(), Style::default()),
                    ]),
                    Line::from(Span::styled(
                        " (content not loaded — press Enter to open)",
                        Style::default().fg(Color::DarkGray),
                    )),
                ]
            }
        } else {
            vec![Line::from(Span::styled(
                " select an artifact",
                Style::default().fg(Color::DarkGray),
            ))]
        };
        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::LEFT)
                .title(" Detail "),
        );
        f.render_widget(para, body[1]);

        match self.popup {
            Popup::Create => self.render_create_popup(f, area),
            Popup::DeleteConfirm => self.render_delete_popup(f, area),
            Popup::RunAgent => self.render_run_popup(f, area, app),
            Popup::RunResult => self.render_run_result(f, area),
            Popup::None => {}
        }
    }

    fn render_create_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 70, 50);
        let kind_label = KINDS[self.create_kind];
        let field_marker = if self.create_field == 0 { "title" } else { "content" };
        let lines = vec![
            Line::from(Span::styled(
                " Create Artifact",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled(" kind: ", Style::default().fg(Color::Cyan)),
                Span::styled(kind_label, Style::default().fg(Color::Yellow)),
                Span::styled(
                    "  (Tab to cycle)",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(" title:", Style::default().fg(Color::Cyan))),
            Line::from(Span::styled(
                if self.create_field == 0 {
                    format!("  {}_", self.create_title)
                } else {
                    format!("  {}", self.create_title)
                },
                Style::default().fg(Color::Reset),
            )),
            Line::from(""),
            Line::from(Span::styled(" content:", Style::default().fg(Color::Cyan))),
            Line::from(Span::styled(
                if self.create_field == 1 {
                    format!("  {}_", self.create_content)
                } else {
                    format!("  {}", self.create_content)
                },
                Style::default().fg(Color::Reset),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(" Tab switch field  Enter create  field: {field_marker}  Esc cancel"),
                Style::default().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Create Artifact "),
            ),
            pop,
        );
    }

    fn render_delete_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 50, 20);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    " Delete this artifact?",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " [y] yes  [n] no",
                    Style::default().fg(Color::Yellow),
                )),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Confirm "),
            ),
            pop,
        );
    }

    fn render_run_popup(&self, f: &mut Frame, area: Rect, app: &AppData) {
        let pop = centered(area, 60, 40);
        let action_label = ACTIONS[self.run_action];
        let mut agent_lines = vec![Line::from(Span::styled(
            format!(" agent: {}  (Tab to cycle)", self.run_agent + 1),
            Style::default().fg(Color::Cyan),
        ))];
        for (i, a) in app.agents.iter().enumerate() {
            let marker = if i == self.run_agent { "▶" } else { " " };
            let host = a.hostname.clone().unwrap_or_else(|| a.id.clone());
            agent_lines.push(Line::from(Span::styled(
                format!("  {marker} {} ({})", host, if a.online { "online" } else { "offline" }),
                Style::default().fg(if a.online { Color::Green } else { Color::DarkGray }),
            )));
        }
        agent_lines.push(Line::from(""));
        agent_lines.push(Line::from(vec![
            Span::styled(" action: ", Style::default().fg(Color::Cyan)),
            Span::styled(action_label, Style::default().fg(Color::Yellow)),
        ]));
        agent_lines.push(Line::from(""));
        if self.run_action > 0 {
            agent_lines.push(Line::from(Span::styled(
                " ⚠ This may create, change, or destroy real infrastructure!",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            agent_lines.push(Line::from(""));
        }
        agent_lines.push(Line::from(Span::styled(
            " Tab cycle agent/action  Enter run  Esc cancel",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(
            Paragraph::new(agent_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Run on Agent "),
            ),
            pop,
        );
    }

    fn render_run_result(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 80, 60);
        let lines: Vec<Line> = if let Some(out) = &self.run_output {
            out.lines()
                .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Green))))
                .collect()
        } else {
            vec![Line::from(Span::styled(
                " running…",
                Style::default().fg(Color::Yellow),
            ))]
        };
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Result (Esc close) "),
            ),
            pop,
        );
    }

    pub async fn handle_key_async(&mut self, key: KeyEvent, app: &mut AppData) {
        match self.popup {
            Popup::Create => {
                self.handle_create_key(key, app).await;
                return;
            }
            Popup::DeleteConfirm => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.delete_artifact(app).await;
                        self.popup = Popup::None;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.popup = Popup::None;
                    }
                    _ => {}
                }
                return;
            }
            Popup::RunAgent => {
                match key.code {
                    KeyCode::Esc => {
                        self.popup = Popup::None;
                    }
                    KeyCode::Tab => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            if self.run_action > 0 {
                                self.run_action -= 1;
                            } else {
                                self.run_action = ACTIONS.len() - 1;
                            }
                        } else {
                            self.run_action = (self.run_action + 1) % ACTIONS.len();
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.run_agent > 0 {
                            self.run_agent -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.run_agent + 1 < app.agents.len() {
                            self.run_agent += 1;
                        }
                    }
                    KeyCode::Enter => {
                        self.run_on_agent(app).await;
                    }
                    _ => {}
                }
                return;
            }
            Popup::RunResult => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.popup = Popup::None;
                    self.run_output = None;
                }
                return;
            }
            Popup::None => {}
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let len = app.artifacts.len();
                if len > 0 {
                    let i = self.state.selected().unwrap_or(0);
                    self.state.select(Some((i + 1).min(len - 1)));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.state.selected().unwrap_or(0);
                self.state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Enter => {
                if let Some(a) = self
                    .state
                    .selected()
                    .and_then(|i| app.artifacts.get(i))
                    .cloned()
                {
                    if a.content.is_none() {
                        if let Ok(full) = app
                            .client
                            .get_json::<ArtifactInfo>(&format!("/artifacts/{}", a.id))
                            .await
                        {
                            self.detail = Some(full);
                        } else {
                            self.detail = Some(a);
                        }
                    } else {
                        self.detail = Some(a);
                    }
                }
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
                self.popup = Popup::Create;
                self.create_title.clear();
                self.create_content.clear();
                self.create_kind = 0;
                self.create_field = 0;
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => {
                self.download_artifact(app).await;
            }
            KeyCode::Char('D') => {
                if self.state.selected().map(|i| app.artifacts.get(i).is_some()).unwrap_or(false) {
                    self.popup = Popup::DeleteConfirm;
                }
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => {
                if let Some(a) = self.state.selected().and_then(|i| app.artifacts.get(i)) {
                    if a.kind == "terraform" || a.kind == "terragrunt" {
                        if !app.agents.is_empty() {
                            self.popup = Popup::RunAgent;
                            self.run_agent = 0;
                            self.run_action = 0;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_create_key(&mut self, key: KeyEvent, app: &mut AppData) {
        match key.code {
            KeyCode::Esc => {
                self.popup = Popup::None;
            }
            KeyCode::Tab => {
                self.create_field = 1 - self.create_field;
            }
            KeyCode::Backspace => {
                if self.create_field == 0 {
                    self.create_title.pop();
                } else {
                    self.create_content.pop();
                }
            }
            KeyCode::Enter => {
                let title = self.create_title.trim().to_string();
                if !title.is_empty() {
                    let kind = KINDS[self.create_kind].to_string();
                    let content = self.create_content.clone();
                    if let Some(pid) = app.current_project_id() {
                        let body = serde_json::json!({
                            "title": title,
                            "kind": kind,
                            "content": content,
                        });
                        let _ = app
                            .client
                            .post_json::<_, serde_json::Value>(
                                &format!("/projects/{pid}/artifacts"),
                                &body,
                            )
                            .await;
                        app.refresh_side_data().await;
                    }
                }
                self.popup = Popup::None;
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                if self.create_field == 0 {
                    self.create_title.push(c);
                } else {
                    self.create_content.push(c);
                }
            }
            _ => {}
        }
    }

    async fn download_artifact(&mut self, app: &mut AppData) {
        if let Some(a) = self.state.selected().and_then(|i| app.artifacts.get(i)) {
            let path = format!("/artifacts/{}/download", a.id);
            match app.client.get_bytes(&path).await {
                Ok(bytes) => {
                    let filename = format!("{}.{}", sanitize(&a.title), ext_for(&a.kind));
                    let _ = std::fs::write(&filename, &bytes);
                    app.status = format!("downloaded: {filename}");
                }
                Err(e) => {
                    app.status = format!("download error: {e}");
                }
            }
        }
    }

    async fn delete_artifact(&mut self, app: &mut AppData) {
        if let Some(a) = self.state.selected().and_then(|i| app.artifacts.get(i)) {
            let _ = app.client.delete(&format!("/artifacts/{}", a.id)).await;
            app.refresh_side_data().await;
            app.status = "artifact deleted".to_string();
        }
    }

    async fn run_on_agent(&mut self, app: &mut AppData) {
        let Some(pid) = app.current_project_id() else {
            return;
        };
        let Some(aid) = app.agents.get(self.run_agent).map(|a| a.id.clone()) else {
            return;
        };
        let Some(art) = self.state.selected().and_then(|i| app.artifacts.get(i)) else {
            return;
        };
        let action = ACTIONS[self.run_action];
        let body = serde_json::json!({
            "agent_id": aid,
            "artifact_id": art.id,
            "action": action,
            "timeout_secs": 300,
        });
        self.popup = Popup::RunResult;
        self.run_output = None;
        let client = app.client.clone();
        let path = format!("/projects/{pid}/agents/{aid}/terraform/{}", art.id);
        match client
            .http
            .post(client.url(&path))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(val) = resp.json::<serde_json::Value>().await {
                        let stdout = val["stdout"].as_str().unwrap_or("");
                        let stderr = val["stderr"].as_str().unwrap_or("");
                        let exit = val["exit_code"].as_i64().unwrap_or(-1);
                        let mut out = String::new();
                        if !stdout.is_empty() {
                            out.push_str(stdout);
                        }
                        if !stderr.is_empty() {
                            out.push_str("\n[stderr]\n");
                            out.push_str(stderr);
                        }
                        out.push_str(&format!("\n[exit {exit}]"));
                        self.run_output = Some(out);
                    }
                } else {
                    self.run_output = Some(format!("error: {}", resp.status()));
                }
            }
            Err(e) => {
                self.run_output = Some(format!("error: {e}"));
            }
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn ext_for(kind: &str) -> &str {
    match kind {
        "terraform" => "tf",
        "terragrunt" => "hcl",
        "bash" => "sh",
        "markdown" => "md",
        "pdf" => "pdf",
        _ => "txt",
    }
}

fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let h = area.height * height_pct / 100;
    let w = area.width * width_pct / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}