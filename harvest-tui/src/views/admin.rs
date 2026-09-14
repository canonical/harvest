use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::AppData;

#[derive(Clone, Copy, PartialEq)]
enum AdminSection {
    Users,
    Groups,
}

#[derive(Clone, Copy, PartialEq)]
enum Popup {
    None,
    CreateGroup,
    DeleteGroupConfirm,
}

pub struct AdminView {
    state: ListState,
    section: AdminSection,
    popup: Popup,
    group_name: String,
    group_desc: String,
    group_field: usize,
}

impl AdminView {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            state,
            section: AdminSection::Users,
            popup: Popup::None,
            group_name: String::new(),
            group_desc: String::new(),
            group_field: 0,
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                " Admin  ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  u users  g groups  c create group",
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        match self.section {
            AdminSection::Users => self.render_users(f, chunks[1], app),
            AdminSection::Groups => self.render_groups(f, chunks[1], app),
        }

        match self.popup {
            Popup::CreateGroup => self.render_create_group_popup(f, area),
            Popup::DeleteGroupConfirm => self.render_delete_group_popup(f, area),
            Popup::None => {}
        }
    }

    fn render_users(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let mut items: Vec<ListItem> = Vec::new();
        for u in &app.users {
            let role_color = if u.role == "admin" { Color::Yellow } else { Color::Reset };
            items.push(ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!(" {:<6} ", u.role),
                        Style::default().fg(role_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(u.email.clone(), Style::default()),
                    Span::styled(
                        format!("   {}", u.name),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
            ]));
        }
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                " no users (admin access required)",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        let list = List::new(items)
            .block(Block::default().borders(Borders::TOP).title(" Users "))
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, area, &mut self.state);
    }

    fn render_groups(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let mut items: Vec<ListItem> = Vec::new();
        for g in &app.groups {
            let default_marker = if g.is_default { " ★" } else { "" };
            items.push(ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!(" {:<20} ", g.name),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} members  ", g.member_count),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        default_marker.to_string(),
                        Style::default().fg(Color::Yellow),
                    ),
                ]),
                Line::from(Span::styled(
                    format!("  {}", g.description),
                    Style::default().fg(Color::DarkGray),
                )),
            ]));
        }
        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                " no groups",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(" Groups (c create, d delete, Space toggle default) "),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, area, &mut self.state);
    }

    fn render_create_group_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 60, 30);
        let fields = ["name", "description"];
        let field_label = fields[self.group_field];
        let lines = vec![
            Line::from(Span::styled(
                " Create Group",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled(" name: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    if self.group_field == 0 {
                        format!("{}_", self.group_name)
                    } else {
                        self.group_name.clone()
                    },
                    Style::default().fg(Color::Reset),
                ),
            ]),
            Line::from(vec![
                Span::styled(" desc: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    if self.group_field == 1 {
                        format!("{}_", self.group_desc)
                    } else {
                        self.group_desc.clone()
                    },
                    Style::default().fg(Color::Reset),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                format!(" Tab switch field  Enter create  field: {field_label}  Esc cancel"),
                Style::default().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Create Group "),
            ),
            pop,
        );
    }

    fn render_delete_group_popup(&self, f: &mut Frame, area: Rect) {
        let pop = centered(area, 50, 20);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    " Delete this group?",
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
            Popup::CreateGroup => {
                match key.code {
                    KeyCode::Esc => {
                        self.popup = Popup::None;
                    }
                    KeyCode::Tab => {
                        self.group_field = 1 - self.group_field;
                    }
                    KeyCode::Backspace => {
                        if self.group_field == 0 {
                            self.group_name.pop();
                        } else {
                            self.group_desc.pop();
                        }
                    }
                    KeyCode::Enter => {
                        let name = self.group_name.trim().to_string();
                        if !name.is_empty() {
                            let body = serde_json::json!({
                                "name": name,
                                "description": self.group_desc.trim(),
                            });
                            let _ = app
                                .client
                                .post_json::<_, serde_json::Value>("/admin/groups", &body)
                                .await;
                            app.refresh_side_data().await;
                        }
                        self.popup = Popup::None;
                    }
                    KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                        if self.group_field == 0 {
                            self.group_name.push(c);
                        } else {
                            self.group_desc.push(c);
                        }
                    }
                    _ => {}
                }
                return;
            }
            Popup::DeleteGroupConfirm => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let Some(g) = self.state.selected().and_then(|i| app.groups.get(i)) {
                            let _ = app
                                .client
                                .delete(&format!("/admin/groups/{}", g.id))
                                .await;
                            app.refresh_side_data().await;
                        }
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
            KeyCode::Char('u') => {
                self.section = AdminSection::Users;
                self.state.select(Some(0));
            }
            KeyCode::Char('g') => {
                self.section = AdminSection::Groups;
                self.state.select(Some(0));
            }
            KeyCode::Char('c') if self.section == AdminSection::Groups => {
                self.popup = Popup::CreateGroup;
                self.group_name.clear();
                self.group_desc.clear();
                self.group_field = 0;
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE && self.section == AdminSection::Groups => {
                if self.state.selected().and_then(|i| app.groups.get(i)).is_some() {
                    self.popup = Popup::DeleteGroupConfirm;
                }
            }
            KeyCode::Char(' ') if self.section == AdminSection::Groups => {
                if let Some(g) = self.state.selected().and_then(|i| app.groups.get(i)) {
                    let body = serde_json::json!({ "is_default": !g.is_default });
                    let _ = app
                        .client
                        .put_json::<_, serde_json::Value>(
                            &format!("/admin/groups/{}/default", g.id),
                            &body,
                        )
                        .await;
                    app.refresh_side_data().await;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = match self.section {
                    AdminSection::Users => app.users.len(),
                    AdminSection::Groups => app.groups.len(),
                };
                if len > 0 {
                    let i = self.state.selected().unwrap_or(0);
                    self.state.select(Some((i + 1).min(len - 1)));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.state.selected().unwrap_or(0);
                self.state.select(Some(i.saturating_sub(1)));
            }
            _ => {}
        }
    }
}

fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let h = area.height * height_pct / 100;
    let w = area.width * width_pct / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}