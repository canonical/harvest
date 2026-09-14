use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::api::DeploymentInfo;
use crate::app::AppData;

pub struct DeployView {
    deployment: Option<DeploymentInfo>,
    loaded: bool,
}

impl DeployView {
    pub fn new() -> Self {
        Self {
            deployment: None,
            loaded: false,
        }
    }

    async fn ensure_loaded(&mut self, app: &mut AppData) {
        if self.loaded {
            return;
        }
        let Some(pid) = app.current_project_id() else {
            return;
        };
        if let Ok(dep) = app
            .client
            .get_json::<DeploymentInfo>(&format!("/projects/{pid}/deployment"))
            .await
        {
            self.deployment = Some(dep);
        }
        self.loaded = true;
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, app: &AppData) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let header = Paragraph::new(Line::from(Span::styled(
            " Deploy & Design",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Min(0)])
            .split(chunks[1]);

        let pid = app.current_project_id().unwrap_or_default();

        let mut design_lines = vec![
            Line::from(Span::styled(
                " Design doc",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        if pid.is_empty() {
            design_lines.push(Line::from(Span::styled(
                "  no project selected",
                Style::default().fg(Color::DarkGray),
            )));
        } else if let Some(dep) = &self.deployment {
            design_lines.push(Line::from(vec![
                Span::styled(" name: ", Style::default().fg(Color::Cyan)),
                Span::styled(dep.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
            ]));
            design_lines.push(Line::from(vec![
                Span::styled(" status: ", Style::default().fg(Color::Cyan)),
                Span::styled(dep.status.clone(), Style::default()),
            ]));
            let infra_color = match dep.infra_state.as_str() {
                "up" => Color::Green,
                "broken" => Color::Red,
                "destroyed" => Color::DarkGray,
                "destroy_failed" => Color::Red,
                _ => Color::DarkGray,
            };
            design_lines.push(Line::from(vec![
                Span::styled(" infra: ", Style::default().fg(Color::Cyan)),
                Span::styled(dep.infra_state.clone(), Style::default().fg(infra_color)),
            ]));
            design_lines.push(Line::from(""));
            design_lines.push(Line::from(vec![
                Span::styled(" artifacts: ", Style::default().fg(Color::Cyan)),
                Span::styled(format!("{}", dep.artifacts.len()), Style::default()),
            ]));
            for a in &dep.artifacts {
                design_lines.push(Line::from(Span::styled(
                    format!("   • {} [{}]", a.title, a.kind),
                    Style::default().fg(Color::Reset),
                )));
            }
        } else {
            design_lines.push(Line::from(Span::styled(
                format!("  project: {pid}"),
                Style::default().fg(Color::DarkGray),
            )));
            design_lines.push(Line::from(""));
            design_lines.push(Line::from(Span::styled(
                "  Use the chat view to drive the deployment pipeline:",
                Style::default().fg(Color::Reset),
            )));
            design_lines.push(Line::from(Span::styled(
                "    • generate a design doc",
                Style::default().fg(Color::Reset),
            )));
            design_lines.push(Line::from(Span::styled(
                "    • provision a terraform bundle",
                Style::default().fg(Color::Reset),
            )));
            design_lines.push(Line::from(Span::styled(
                "    • propose / approve bundle changes",
                Style::default().fg(Color::Reset),
            )));
            design_lines.push(Line::from(Span::styled(
                "    • run plan / apply / destroy",
                Style::default().fg(Color::Reset),
            )));
        }
        design_lines.push(Line::from(""));
        design_lines.push(Line::from(Span::styled(
            "  Artifacts produced appear in the Artifacts tab.",
            Style::default().fg(Color::DarkGray),
        )));

        let para = Paragraph::new(design_lines).block(
            Block::default()
                .borders(Borders::RIGHT)
                .title(" DESIGN "),
        );
        f.render_widget(para, body[0]);

        let plan_lines = if let Some(dep) = &self.deployment {
            let mut lines = vec![
                Line::from(Span::styled(
                    " Execution plan",
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(" status: ", Style::default().fg(Color::Cyan)),
                    Span::styled(dep.status.clone(), Style::default()),
                ]),
                Line::from(vec![
                    Span::styled(" infra_state: ", Style::default().fg(Color::Cyan)),
                    Span::styled(dep.infra_state.clone(), Style::default()),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press Ctrl-R to refresh.",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            if dep.infra_state == "broken" || dep.infra_state == "destroy_failed" {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  ⚠ Infrastructure is in a broken state!",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )));
            }
            lines
        } else {
            vec![
                Line::from(Span::styled(
                    " Execution plan",
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  (no deployment yet)",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Runs are streamed from the agent and",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "  shown here once a deployment is active.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        };
        let plan = Paragraph::new(plan_lines).block(
            Block::default()
                .borders(Borders::LEFT)
                .title(" PLAN / RUNS "),
        );
        f.render_widget(plan, body[1]);
    }

    pub async fn handle_key_async(&mut self, _key: KeyEvent, app: &mut AppData) {
        self.ensure_loaded(app).await;
    }
}