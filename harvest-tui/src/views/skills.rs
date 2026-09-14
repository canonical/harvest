use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::api::SkillInfo;
use crate::app::AppData;
use crate::widgets::markdown::render_markdown;

#[derive(Clone, Copy, PartialEq)]
enum SkillTab {
    Project,
    Global,
}

#[derive(Clone, Copy, PartialEq)]
enum Popup {
    None,
    Create,
    Edit,
    DeleteConfirm,
}

pub struct SkillsView {
    state: ListState,
    detail: Option<SkillInfo>,
    tab: SkillTab,
    popup: Popup,
    edit_name: String,
    edit_desc: String,
    edit_content: String,
    edit_field: usize,
    editing_id: Option<String>,
}

impl SkillsView {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            state,
            detail: None,
            tab: SkillTab::Project,
            popup: Popup::None,
            edit_name: String::new(),
            edit_desc: String::new(),
            edit_content: String::new(),
            edit_field: 0,
            editing_id: None,
        }
    }

    fn current_skills<'a>(&self, app: &'a AppData) -> Vec<&'a SkillInfo> {
        match self.tab {
            SkillTab::Project => app.skills.iter().filter(|s| !s.id.starts_with("global")).collect(),
            SkillTab::Global => app.skills.iter().collect(),
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let tab_label = match self.tab {
            SkillTab::Project => "Project",
            SkillTab::Global => "Global",
        };
        let skills = self.current_skills(app);
        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" Skills [{}]  [ {} ]", skills.len(), tab_label),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  (Tab switch project/global, c create, e edit, D delete)",
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Min(0)])
            .split(chunks[1]);

        let mut items: Vec<ListItem> = Vec::new();
        for s in &skills {
            items.push(ListItem::new(vec![
                Line::from(Span::styled(
                    s.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    truncate(&s.description, 40),
                    Style::default().fg(Color::DarkGray),
                )),
            ]));
        }
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                " no skills",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        let list = List::new(items)
            .block(Block::default().borders(Borders::RIGHT).title(" List "))
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
            .and_then(|i| skills.get(i))
            .copied()
            .cloned()
            .or_else(|| self.detail.clone());

        let lines: Vec<Line> = if let Some(s) = &detail {
            let mut out = vec![
                Line::from(vec![
                    Span::styled(" name: ", Style::default().fg(Color::Cyan)),
                    Span::styled(s.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled(" updated: ", Style::default().fg(Color::Cyan)),
                    Span::styled(s.updated_at.clone(), Style::default()),
                ]),
                Line::from(""),
            ];
            if let Some(content) = &s.content {
                let w = body[1].width as usize;
                let mut rendered = render_markdown(content, w);
                out.append(&mut rendered);
            } else {
                out.push(Line::from(Span::styled(
                    " (press Enter to load content)",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            out
        } else {
            vec![Line::from(Span::styled(
                " select a skill",
                Style::default().fg(Color::DarkGray),
            ))]
        };
        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::LEFT)
                .title(" Body "),
        );
        f.render_widget(para, body[1]);

        match self.popup {
            Popup::Create | Popup::Edit => self.render_edit_popup(f, area),
            Popup::DeleteConfirm => self.render_delete_popup(f, area),
            Popup::None => {}
        }
    }

    fn render_edit_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 70, 60);
        let title = if self.popup == Popup::Create {
            " Create Skill"
        } else {
            " Edit Skill"
        };
        let fields = ["name", "description", "content"];
        let field_label = fields[self.edit_field];
        let lines = vec![
            Line::from(Span::styled(
                title,
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled(" name: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    if self.edit_field == 0 {
                        format!("{}_", self.edit_name)
                    } else {
                        self.edit_name.clone()
                    },
                    Style::default().fg(Color::Reset),
                ),
            ]),
            Line::from(vec![
                Span::styled(" desc: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    if self.edit_field == 1 {
                        format!("{}_", self.edit_desc)
                    } else {
                        self.edit_desc.clone()
                    },
                    Style::default().fg(Color::Reset),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(" content:", Style::default().fg(Color::Cyan))),
            Line::from(Span::styled(
                if self.edit_field == 2 {
                    format!("  {}_", self.edit_content)
                } else {
                    format!("  {}", self.edit_content)
                },
                Style::default().fg(Color::Reset),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(" Tab switch field  Enter save  field: {field_label}  Esc cancel"),
                Style::default().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", title.trim())),
            ),
            pop,
        );
    }

    fn render_delete_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 50, 20);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    " Delete this skill?",
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
            Popup::Create | Popup::Edit => {
                self.handle_edit_key(key, app).await;
                return;
            }
            Popup::DeleteConfirm => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.delete_skill(app).await;
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

        let skills = self.current_skills(app);

        match key.code {
            KeyCode::Tab => {
                self.tab = if self.tab == SkillTab::Project {
                    SkillTab::Global
                } else {
                    SkillTab::Project
                };
                self.state.select(Some(0));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = skills.len();
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
                if let Some(s) = self.state.selected().and_then(|i| skills.get(i)) {
                    let s = (*s).clone();
                    if s.content.is_none() {
                        let id = s.id.clone();
                        if let Ok(full) = app.client.get_json::<SkillInfo>(&format!("/skills/{id}")).await {
                            self.detail = Some(full);
                        } else {
                            self.detail = Some(s);
                        }
                    } else {
                        self.detail = Some(s);
                    }
                }
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
                self.popup = Popup::Create;
                self.edit_name.clear();
                self.edit_desc.clear();
                self.edit_content.clear();
                self.edit_field = 0;
                self.editing_id = None;
            }
            KeyCode::Char('e') if key.modifiers == KeyModifiers::NONE => {
                if let Some(s) = self.state.selected().and_then(|i| skills.get(i)) {
                    let s = (*s).clone();
                    let id = s.id.clone();
                    let name = s.name.clone();
                    let desc = s.description.clone();
                    let content = s.content.clone().unwrap_or_default();
                    self.edit_name = name;
                    self.edit_desc = desc;
                    self.edit_content = content;
                    self.edit_field = 0;
                    self.editing_id = Some(id);
                    self.popup = Popup::Edit;
                }
            }
            KeyCode::Char('D') => {
                if self.state.selected().and_then(|i| skills.get(i)).is_some() {
                    self.popup = Popup::DeleteConfirm;
                }
            }
            _ => {}
        }
    }

    async fn handle_edit_key(&mut self, key: KeyEvent, app: &mut AppData) {
        match key.code {
            KeyCode::Esc => {
                self.popup = Popup::None;
            }
            KeyCode::Tab => {
                self.edit_field = (self.edit_field + 1) % 3;
            }
            KeyCode::Backspace => {
                match self.edit_field {
                    0 => self.edit_name.pop(),
                    1 => self.edit_desc.pop(),
                    _ => self.edit_content.pop(),
                };
            }
            KeyCode::Enter => {
                let name = self.edit_name.trim().to_string();
                if !name.is_empty() {
                    let body = serde_json::json!({
                        "name": name,
                        "description": self.edit_desc.trim(),
                        "content": self.edit_content.clone(),
                    });
                    if let Some(id) = self.editing_id.take() {
                        let _ = app
                            .client
                            .put_json::<_, serde_json::Value>(&format!("/skills/{id}"), &body)
                            .await;
                    } else if let Some(pid) = app.current_project_id() {
                        let _ = app
                            .client
                            .post_json::<_, serde_json::Value>(
                                &format!("/projects/{pid}/skills"),
                                &body,
                            )
                            .await;
                    }
                    app.refresh_side_data().await;
                }
                self.popup = Popup::None;
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                match self.edit_field {
                    0 => self.edit_name.push(c),
                    1 => self.edit_desc.push(c),
                    _ => self.edit_content.push(c),
                }
            }
            _ => {}
        }
    }

    async fn delete_skill(&mut self, app: &mut AppData) {
        let skills = self.current_skills(app);
        if let Some(s) = self.state.selected().and_then(|i| skills.get(i)) {
            let id = s.id.clone();
            let _ = app.client.delete(&format!("/skills/{id}")).await;
            app.refresh_side_data().await;
            app.status = "skill deleted".to_string();
        }
    }
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