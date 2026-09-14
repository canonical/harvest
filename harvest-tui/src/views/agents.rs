use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::api::AgentInfo;
use crate::app::AppData;

#[derive(Clone, Copy, PartialEq)]
pub enum Popup {
    None,
    Exec,
    Add,
    PortForwards,
    DeleteConfirm,
}

pub struct AgentsView {
    state: ListState,
    detail: Option<AgentInfo>,
    pub popup: Popup,
    exec_input: String,
    exec_output: Option<String>,
    exec_running: bool,
    add_input: String,
    add_desc: String,
    add_output: Option<String>,
    pf_state: ListState,
    pf_input: String,
    pf_route: String,
    pf_editing: bool,
    pf_forwards: Vec<crate::api::PortForward>,
}

impl AgentsView {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            state,
            detail: None,
            popup: Popup::None,
            exec_input: String::new(),
            exec_output: None,
            exec_running: false,
            add_input: String::new(),
            add_desc: String::new(),
            add_output: None,
            pf_state: ListState::default(),
            pf_input: String::new(),
            pf_route: String::new(),
            pf_editing: false,
            pf_forwards: Vec::new(),
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let online = app.agents.iter().filter(|a| a.online).count();
        let header = Paragraph::new(Line::from(Span::styled(
            format!(" Agents  [ {online} online / {} total ]", app.agents.len()),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Min(0)])
            .split(chunks[1]);

        let mut items: Vec<ListItem> = Vec::new();
        for a in &app.agents {
            let dot = if a.online { "●" } else { "○" };
            let color = if a.online { Color::Green } else { Color::DarkGray };
            let host = a.hostname.clone().unwrap_or_else(|| a.id.clone());
            items.push(ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!(" {dot} "), Style::default().fg(color)),
                    Span::styled(host.clone(), Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!("  {}", a.provider),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::from(Span::styled(
                    format!(
                        "   last seen: {}",
                        a.last_seen.as_deref().unwrap_or("?")
                    ),
                    Style::default().fg(Color::DarkGray),
                )),
            ]));
        }
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                " no agents connected",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .title(" Connected "),
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
            .and_then(|i| app.agents.get(i))
            .cloned()
            .or_else(|| self.detail.clone());

        let lines = if let Some(a) = &detail {
            vec![
                Line::from(vec![
                    Span::styled(" id: ", Style::default().fg(Color::Cyan)),
                    Span::styled(a.id.clone(), Style::default()),
                ]),
                Line::from(vec![
                    Span::styled(" hostname: ", Style::default().fg(Color::Cyan)),
                    Span::styled(a.hostname.clone().unwrap_or_default(), Style::default()),
                ]),
                Line::from(vec![
                    Span::styled(" online: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        if a.online { "yes" } else { "no" }.to_string(),
                        Style::default().fg(if a.online { Color::Green } else { Color::DarkGray }),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(" provider: ", Style::default().fg(Color::Cyan)),
                    Span::styled(a.provider.clone(), Style::default()),
                ]),
                Line::from(vec![
                    Span::styled(" last_seen: ", Style::default().fg(Color::Cyan)),
                    Span::styled(a.last_seen.clone().unwrap_or_default(), Style::default()),
                ]),
                Line::from(vec![
                    Span::styled(" created: ", Style::default().fg(Color::Cyan)),
                    Span::styled(a.created_at.clone().unwrap_or_default(), Style::default()),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    if let Some(d) = &a.description {
                        d.clone()
                    } else {
                        String::new()
                    },
                    Style::default().fg(Color::Reset),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " x execute  a add  r restart  d delete  f port-fwd  Enter select",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        } else {
            vec![Line::from(Span::styled(
                " select an agent",
                Style::default().fg(Color::DarkGray),
            ))]
        };
        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::LEFT)
                .title(" Details "),
        );
        f.render_widget(para, body[1]);

        match self.popup {
            Popup::Exec => self.render_exec_popup(f, area),
            Popup::Add => self.render_add_popup(f, area),
            Popup::PortForwards => self.render_pf_popup(f, area),
            Popup::DeleteConfirm => self.render_delete_popup(f, area),
            Popup::None => {}
        }
    }

    fn render_exec_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 80, 60);
        let output = self.exec_output.clone().unwrap_or_default();
        let out_lines: Vec<Line> = if output.is_empty() {
            if self.exec_running {
                vec![Line::from(Span::styled(
                    " running…",
                    Style::default().fg(Color::Yellow),
                ))]
            } else {
                vec![Line::from(Span::styled(
                    " (no output)",
                    Style::default().fg(Color::DarkGray),
                ))]
            }
        } else {
            output
                .lines()
                .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Green))))
                .collect()
        };
        let mut all = out_lines;
        all.push(Line::from(""));
        all.push(Line::from(Span::styled(
            format!(" $ {}_", self.exec_input),
            Style::default().fg(Color::Cyan),
        )));
        f.render_widget(
            Paragraph::new(all).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Execute (Esc close, Enter run) "),
            ),
            pop,
        );
    }

    fn render_add_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 80, 50);
        let lines = if let Some(out) = &self.add_output {
            out.lines()
                .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Green))))
                .collect::<Vec<_>>()
        } else {
            vec![
                Line::from(Span::styled(
                    " Add agent (manual install)",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " name:",
                    Style::default().fg(Color::Cyan),
                )),
                Line::from(Span::styled(
                    format!("  {}_", self.add_input),
                    Style::default().fg(Color::Reset),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " description:",
                    Style::default().fg(Color::Cyan),
                )),
                Line::from(Span::styled(
                    format!("  {}_", self.add_desc),
                    Style::default().fg(Color::Reset),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " Enter create  Esc cancel",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        };
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Add Agent "),
            ),
            pop,
        );
    }

    fn render_pf_popup(&mut self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 70, 60);
        let mut items: Vec<ListItem> = Vec::new();
        for pf in &self.pf_forwards {
            items.push(ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!(" {}", pf.port),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {}", pf.route_name),
                        Style::default().fg(Color::Reset),
                    ),
                ]),
                Line::from(Span::styled(
                    format!("  {}", pf.route_url),
                    Style::default().fg(Color::DarkGray),
                )),
            ]));
        }
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                " no port forwards",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        let mut all_lines: Vec<Line> = Vec::new();
        if self.pf_editing {
            all_lines.push(Line::from(Span::styled(
                format!(" route: {}_", self.pf_route),
                Style::default().fg(Color::Cyan),
            )));
            all_lines.push(Line::from(Span::styled(
                format!(" port: {}_", self.pf_input),
                Style::default().fg(Color::Cyan),
            )));
            all_lines.push(Line::from(Span::styled(
                " Enter save  Esc cancel",
                Style::default().fg(Color::DarkGray),
            )));
            all_lines.push(Line::from(""));
        }
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Port Forwards (Esc close, a add, d delete) "),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, pop, &mut self.pf_state);
    }

    fn render_delete_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 50, 20);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    " Delete this agent?",
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

    pub async fn handle_key_async(&mut self, key: KeyEvent, app: &mut AppData) {
        match self.popup {
            Popup::Exec => {
                self.handle_exec_key(key, app).await;
                return;
            }
            Popup::Add => {
                self.handle_add_key(key, app).await;
                return;
            }
            Popup::PortForwards => {
                self.handle_pf_key(key, app).await;
                return;
            }
            Popup::DeleteConfirm => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.delete_agent(app).await;
                        self.popup = Popup::None;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.popup = Popup::None;
                    }
                    _ => {}
                }
                return;
            }
            Popup::None => {}
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let len = app.agents.len();
                if len > 0 {
                    let i = self.state.selected().unwrap_or(0);
                    self.state.select(Some((i + 1).min(len - 1)));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.state.selected().unwrap_or(0);
                self.state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Char('x') => {
                if self.state.selected().map(|i| app.agents.get(i).is_some()).unwrap_or(false) {
                    self.popup = Popup::Exec;
                    self.exec_input.clear();
                    self.exec_output = None;
                }
            }
            KeyCode::Char('a') => {
                self.popup = Popup::Add;
                self.add_input.clear();
                self.add_desc.clear();
                self.add_output = None;
            }
            KeyCode::Char('r') => {
                self.restart_agent(app).await;
            }
            KeyCode::Char('d') => {
                if self.state.selected().map(|i| app.agents.get(i).is_some()).unwrap_or(false) {
                    self.popup = Popup::DeleteConfirm;
                }
            }
            KeyCode::Char('f') => {
                if let Some(aid) = self
                    .state
                    .selected()
                    .and_then(|i| app.agents.get(i).map(|a| a.id.clone()))
                {
                    if let Some(pid) = app.current_project_id() {
                        if let Ok(pfs) = app
                            .client
                            .get_json::<Vec<crate::api::PortForward>>(&format!(
                                "/projects/{pid}/agents/{aid}/port-forwards"
                            ))
                            .await
                        {
                            self.pf_forwards = pfs;
                        }
                    }
                    self.popup = Popup::PortForwards;
                    self.pf_state.select(Some(0));
                }
            }
            KeyCode::Enter => {
                if let Some(a) = self.state.selected().and_then(|i| app.agents.get(i)) {
                    self.detail = Some(a.clone());
                }
            }
            _ => {}
        }
    }

    async fn handle_exec_key(&mut self, key: KeyEvent, app: &mut AppData) {
        match key.code {
            KeyCode::Esc => {
                self.popup = Popup::None;
                self.exec_input.clear();
                self.exec_output = None;
            }
            KeyCode::Enter => {
                if !self.exec_running {
                    self.run_command(app).await;
                }
            }
            KeyCode::Backspace => {
                self.exec_input.pop();
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                self.exec_input.push(c);
            }
            _ => {}
        }
    }

    async fn handle_add_key(&mut self, key: KeyEvent, app: &mut AppData) {
        if self.add_output.is_some() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                self.popup = Popup::None;
                self.add_output = None;
            }
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.popup = Popup::None;
            }
            KeyCode::Enter => {
                let name = self.add_input.trim().to_string();
                if !name.is_empty() {
                    if let Some(pid) = app.current_project_id() {
                        let body = serde_json::json!({
                            "name": name,
                            "description": self.add_desc.trim(),
                        });
                        match app
                            .client
                            .post_json::<_, serde_json::Value>(
                                &format!("/projects/{pid}/agents/install-token"),
                                &body,
                            )
                            .await
                        {
                            Ok(resp) => {
                                let token = resp.get("install_token").and_then(|v| v.as_str()).unwrap_or("");
                                let cmd = resp.get("install_command").and_then(|v| v.as_str()).unwrap_or("");
                                if !cmd.is_empty() {
                                    self.add_output = Some(format!("Install command:\n{cmd}\n\nToken: {token}"));
                                } else {
                                    self.add_output = Some(format!("Install token: {token}\n\nRun on the target machine:\ncurl -sSL <server>/install | TOKEN={token} bash"));
                                }
                            }
                            Err(e) => {
                                self.add_output = Some(format!("error: {e}"));
                            }
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if self.add_desc.is_empty() || !self.add_input.is_empty() {
                    self.add_input.pop();
                } else {
                    self.add_desc.pop();
                }
            }
            KeyCode::Tab => {
                // switch between name and desc
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                self.add_input.push(c);
            }
            _ => {}
        }
    }

    async fn handle_pf_key(&mut self, key: KeyEvent, app: &mut AppData) {
        if self.pf_editing {
            match key.code {
                KeyCode::Esc => {
                    self.pf_editing = false;
                    self.pf_input.clear();
                    self.pf_route.clear();
                }
                KeyCode::Tab => {}
                KeyCode::Enter => {
                    let port: i64 = self.pf_input.trim().parse().unwrap_or(0);
                    let route = self.pf_route.trim().to_string();
                    if port > 0 && !route.is_empty() {
                        if let (Some(pid), Some(aid)) = (
                            app.current_project_id(),
                            self.state
                                .selected()
                                .and_then(|i| app.agents.get(i).map(|a| a.id.clone())),
                        ) {
                            let body = serde_json::json!({ "port": port, "route_name": route });
                            let _ = app
                                .client
                                .post_json::<_, serde_json::Value>(
                                    &format!("/projects/{pid}/agents/{aid}/port-forwards"),
                                    &body,
                                )
                                .await;
                            if let Ok(pfs) = app
                                .client
                                .get_json::<Vec<crate::api::PortForward>>(&format!(
                                    "/projects/{pid}/agents/{aid}/port-forwards"
                                ))
                                .await
                            {
                                self.pf_forwards = pfs;
                            }
                        }
                    }
                    self.pf_editing = false;
                    self.pf_input.clear();
                    self.pf_route.clear();
                }
                KeyCode::Backspace => {
                    if self.pf_input.is_empty() {
                        self.pf_route.pop();
                    } else {
                        self.pf_input.pop();
                    }
                }
                KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                    self.pf_input.push(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.popup = Popup::None;
            }
            KeyCode::Char('a') => {
                self.pf_editing = true;
                self.pf_input.clear();
                self.pf_route.clear();
            }
            KeyCode::Char('d') => {
                if let Some(idx) = self.pf_state.selected() {
                    if let Some(pf) = self.pf_forwards.get(idx) {
                        let fid = pf.id.clone();
                        if let (Some(pid), Some(aid)) = (
                            app.current_project_id(),
                            self.state
                                .selected()
                                .and_then(|i| app.agents.get(i).map(|a| a.id.clone())),
                        ) {
                            let _ = app
                                .client
                                .delete(&format!(
                                    "/projects/{pid}/agents/{aid}/port-forwards/{fid}"
                                ))
                                .await;
                            if let Ok(pfs) = app
                                .client
                                .get_json::<Vec<crate::api::PortForward>>(&format!(
                                    "/projects/{pid}/agents/{aid}/port-forwards"
                                ))
                                .await
                            {
                                self.pf_forwards = pfs;
                            }
                        }
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.pf_forwards.len();
                if len > 0 {
                    let i = self.pf_state.selected().unwrap_or(0);
                    self.pf_state.select(Some((i + 1).min(len - 1)));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.pf_state.selected().unwrap_or(0);
                self.pf_state.select(Some(i.saturating_sub(1)));
            }
            _ => {}
        }
    }

    async fn run_command(&mut self, app: &mut AppData) {
        let Some(pid) = app.current_project_id() else {
            self.exec_output = Some("no project".to_string());
            return;
        };
        let Some(aid) = self
            .state
            .selected()
            .and_then(|i| app.agents.get(i).map(|a| a.id.clone()))
        else {
            self.exec_output = Some("no agent".to_string());
            return;
        };
        let cmd = self.exec_input.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        self.exec_running = true;
        self.exec_output = Some("running…".to_string());
        let client = app.client.clone();
        let body = serde_json::json!({ "command": cmd, "timeout_secs": 30 });
        let path = format!("/projects/{pid}/agents/{aid}/execute");
        let result = client
            .http
            .post(client.url(&path))
            .json(&body)
            .send()
            .await;
        self.exec_running = false;
        match result {
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
                        self.exec_output = Some(out);
                    }
                } else {
                    self.exec_output = Some(format!("error: {}", resp.status()));
                }
            }
            Err(e) => {
                self.exec_output = Some(format!("error: {e}"));
            }
        }
        self.exec_input.clear();
    }

    async fn restart_agent(&mut self, app: &mut AppData) {
        let Some(pid) = app.current_project_id() else {
            return;
        };
        let Some(aid) = self
            .state
            .selected()
            .and_then(|i| app.agents.get(i).map(|a| a.id.clone()))
        else {
            return;
        };
        let path = format!("/projects/{pid}/agents/{aid}/restart");
        let _ = app.client.post_text(&path, &serde_json::json!({})).await;
        app.status = "restart requested".to_string();
    }

    async fn delete_agent(&mut self, app: &mut AppData) {
        let Some(pid) = app.current_project_id() else {
            return;
        };
        let Some(aid) = self
            .state
            .selected()
            .and_then(|i| app.agents.get(i).map(|a| a.id.clone()))
        else {
            return;
        };
        let path = format!("/projects/{pid}/agents/{aid}");
        let _ = app.client.delete(&path).await;
        app.refresh_side_data().await;
        app.status = "agent deleted".to_string();
    }
}

fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let h = area.height * height_pct / 100;
    let w = area.width * width_pct / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}