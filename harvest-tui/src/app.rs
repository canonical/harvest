use std::io::Stdout;

use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, event::DisableMouseCapture, event::EnableMouseCapture};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::api::{
    AgentInfo, ArtifactInfo, Client, ClientConfig, ConversationSummary, GraphData, GroupInfo,
    MeResponse, ProjectInfo, ProvidersResponse, RepositoryInfo, SkillInfo, UserInfo,
};
use crate::views::{
    admin::AdminView, agents::AgentsView, artifacts::ArtifactsView, chat::ChatView,
    deploy::DeployView, graph::GraphView, skills::SkillsView,
};

pub enum AppEvent {
    Chat(crate::api::AgentEvent),
    ChatError(String),
    Refresh,
}

const VIEW_TITLES: [&str; 7] = [
    "Chat",
    "Graph",
    "Deploy",
    "Agents",
    "Artifacts",
    "Skills",
    "Admin",
];

#[derive(Clone, Copy, PartialEq)]
pub enum SidebarSection {
    Projects,
    Models,
}

pub struct AppData {
    pub client: Client,
    pub projects: Vec<ProjectInfo>,
    pub repositories: Vec<RepositoryInfo>,
    pub providers: Vec<crate::api::ProviderInfo>,
    pub agents: Vec<AgentInfo>,
    pub artifacts: Vec<ArtifactInfo>,
    pub skills: Vec<SkillInfo>,
    pub users: Vec<UserInfo>,
    pub groups: Vec<GroupInfo>,
    pub me: Option<MeResponse>,
    pub current_project: Option<usize>,
    pub conversations: Vec<ConversationSummary>,
    pub graphs: std::collections::HashMap<String, GraphData>,
    pub sidebar_state: ListState,
    pub model_state: ListState,
    pub status: String,
    pub model_pick: Option<(String, String)>,
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
    pub model_pairs: Vec<(String, String, String)>,
}

impl AppData {
    pub fn current_project_id(&self) -> Option<String> {
        self.current_project
            .and_then(|i| self.projects.get(i).map(|p| p.id.clone()))
    }

    pub fn current_project_name(&self) -> &str {
        self.current_project
            .and_then(|i| self.projects.get(i).map(|p| p.name.as_str()))
            .unwrap_or("(no project)")
    }

    pub fn current_repo_and_version(&self) -> Option<(String, String)> {
        let repo = self.repositories.first()?;
        let version = repo.versions.first()?;
        Some((repo.name.clone(), version.clone()))
    }

    pub fn status_line(&self) -> String {
        let proj = self.current_project_name();
        let model = self
            .model_pick
            .as_ref()
            .map(|(_, m)| m.clone())
            .unwrap_or_else(|| "default".to_string());
        format!(" project: {proj}  model: {model}  {} ", self.status)
    }

    pub fn is_admin(&self) -> bool {
        self.me.as_ref().map(|m| m.is_admin).unwrap_or(false)
    }

    pub async fn refresh_side_data(&mut self) {
        if let Some(pid) = self.current_project_id() {
            if let Ok(agents) = self
                .client
                .get_json::<Vec<AgentInfo>>(&format!("/projects/{pid}/agents"))
                .await
            {
                self.agents = agents;
            }
            if let Ok(arts) = self
                .client
                .get_json::<Vec<ArtifactInfo>>(&format!("/projects/{pid}/artifacts"))
                .await
            {
                self.artifacts = arts;
            }
            if let Ok(skills) = self
                .client
                .get_json::<Vec<SkillInfo>>(&format!("/projects/{pid}/skills"))
                .await
            {
                self.skills = skills;
            }
            if let Ok(convs) = self
                .client
                .get_json::<Vec<ConversationSummary>>(&format!("/projects/{pid}/conversations"))
                .await
            {
                self.conversations = convs;
            }
        }
        if let Ok(skills) = self.client.get_json::<Vec<SkillInfo>>("/skills").await {
            if skills.len() > self.skills.len() {
                self.skills = skills;
            }
        }
        if let Ok(users) = self.client.get_json::<Vec<UserInfo>>("/admin/users").await {
            self.users = users;
        }
        if let Ok(groups) = self.client.get_json::<Vec<GroupInfo>>("/admin/groups").await {
            self.groups = groups;
        }
    }

    pub fn rebuild_model_pairs(&mut self) {
        self.model_pairs.clear();
        for p in &self.providers {
            for m in &p.models {
                let mlabel = m.display_name.clone().unwrap_or_else(|| m.id.clone());
                self.model_pairs
                    .push((p.id.clone(), m.id.clone(), mlabel));
            }
        }
    }

    pub fn spawn_stream(&self, body: crate::api::QueryRequest) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let stream = crate::api::stream_query(&client, "/query/stream", &body);
            tokio::pin!(stream);
            while let Some(item) = stream.next().await {
                match item {
                    Ok(ev) => {
                        if tx.send(AppEvent::Chat(ev)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::ChatError(e.to_string()));
                        break;
                    }
                }
            }
        });
    }
}

pub struct App {
    pub data: AppData,
    pub chat: ChatView,
    pub graph: GraphView,
    pub deploy: DeployView,
    pub agents: AgentsView,
    pub artifacts: ArtifactsView,
    pub skills: SkillsView,
    pub admin: AdminView,
    pub active_view: usize,
    pub event_rx: mpsc::UnboundedReceiver<AppEvent>,
    pub should_quit: bool,
    pub show_help: bool,
    pub sidebar_focused: bool,
    pub sidebar_section: SidebarSection,
}

impl App {
    pub async fn new(cfg: ClientConfig) -> Result<Self> {
        let client = Client::new(cfg)?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let projects: Vec<ProjectInfo> = client.get_json("/projects").await.unwrap_or_default();
        let repositories: Vec<RepositoryInfo> =
            client.get_json("/repositories").await.unwrap_or_default();
        let providers: ProvidersResponse = client
            .get_json("/llm/providers")
            .await
            .unwrap_or(ProvidersResponse { providers: Vec::new() });

        let current_project = if projects.is_empty() { None } else { Some(0) };

        let mut sidebar_state = ListState::default();
        sidebar_state.select(current_project);

        let mut model_state = ListState::default();
        model_state.select(Some(0));

        let me = client
            .get_json::<MeResponse>("/auth/me")
            .await
            .ok();

        let mut data = AppData {
            client,
            projects,
            repositories,
            providers: providers.providers,
            agents: Vec::new(),
            artifacts: Vec::new(),
            skills: Vec::new(),
            users: Vec::new(),
            groups: Vec::new(),
            me,
            current_project,
            conversations: Vec::new(),
            graphs: std::collections::HashMap::new(),
            sidebar_state,
            model_state,
            status: String::from("ready"),
            model_pick: None,
            event_tx: event_tx.clone(),
            model_pairs: Vec::new(),
        };
        data.rebuild_model_pairs();
        data.refresh_side_data().await;

        Ok(Self {
            data,
            chat: ChatView::new(),
            graph: GraphView::new(),
            deploy: DeployView::new(),
            agents: AgentsView::new(),
            artifacts: ArtifactsView::new(),
            skills: SkillsView::new(),
            admin: AdminView::new(),
            active_view: 0,
            event_rx,
            should_quit: false,
            show_help: false,
            sidebar_focused: false,
            sidebar_section: SidebarSection::Projects,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.run_loop(&mut terminal).await;

        disable_raw_mode()?;
        execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;

        result
    }

    async fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let mut event_stream = crossterm::event::EventStream::new();

        loop {
            terminal.draw(|f| self.draw(f))?;

            tokio::select! {
                maybe_ev = event_stream.next() => {
                    if let Some(Ok(ev)) = maybe_ev {
                        if let CrosstermEvent::Key(key) = ev {
                            self.handle_key(key).await;
                        }
                    }
                }
                maybe_app = self.event_rx.recv() => {
                    if let Some(app_ev) = maybe_app {
                        self.handle_app_event(app_ev).await;
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    async fn handle_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Chat(agent_ev) => {
                self.chat.on_agent_event(&mut self.data, agent_ev);
            }
            AppEvent::ChatError(msg) => {
                self.chat.on_chat_error(&mut self.data, msg);
            }
            AppEvent::Refresh => {
                self.data.status = "refreshing…".to_string();
                self.data.refresh_side_data().await;
                self.data.status = "ready".to_string();
            }
        }
    }

    fn view_accepts_text(&self) -> bool {
        match self.active_view {
            0 => true,
            1 => self.graph.searching || self.graph.source_pop.is_some(),
            3 => self.agents.popup != crate::views::agents::Popup::None,
            _ => false,
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        if self.show_help {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter) {
                self.show_help = false;
            }
            return;
        }

        if key.code == KeyCode::Char('q')
            && key.modifiers == KeyModifiers::NONE
            && !self.sidebar_focused
            && !self.view_accepts_text()
        {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::F(1) {
            self.show_help = true;
            return;
        }
        if key.code == KeyCode::Char('?') && !self.view_accepts_text() && !self.sidebar_focused {
            self.show_help = true;
            return;
        }

        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.sidebar_focused = !self.sidebar_focused;
            return;
        }

        if self.sidebar_focused {
            self.handle_sidebar_key(key).await;
            return;
        }

        if key.code == KeyCode::Char('r')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !self.view_accepts_text()
        {
            let tx = self.data.event_tx.clone();
            let _ = tx.send(AppEvent::Refresh);
            return;
        }

        if key.code == KeyCode::Tab {
            self.active_view = (self.active_view + 1) % 7;
            return;
        }
        if key.code == KeyCode::BackTab {
            if self.active_view == 0 {
                self.active_view = 6;
            } else {
                self.active_view -= 1;
            }
            return;
        }
        if let KeyCode::Char(c) = key.code {
            if let Some(idx) = c.to_digit(10) {
                if idx >= 1 && (idx as usize) <= 7 && !self.view_accepts_text() {
                    self.active_view = (idx - 1) as usize;
                    return;
                }
            }
        }

        match self.active_view {
            0 => self.chat.handle_key_async(key, &mut self.data).await,
            1 => self.graph.handle_key_async(key, &mut self.data).await,
            2 => self.deploy.handle_key_async(key, &mut self.data).await,
            3 => self.agents.handle_key_async(key, &mut self.data).await,
            4 => self.artifacts.handle_key_async(key, &mut self.data).await,
            5 => self.skills.handle_key_async(key, &mut self.data).await,
            6 => self.admin.handle_key_async(key, &mut self.data).await,
            _ => {}
        }
    }

    async fn handle_sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => {
                self.sidebar_section = if self.sidebar_section == SidebarSection::Projects {
                    SidebarSection::Models
                } else {
                    SidebarSection::Projects
                };
                self.update_sidebar_selection();
            }
            KeyCode::Esc | KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.sidebar_focused = false;
            }
            KeyCode::Esc => {
                self.sidebar_focused = false;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match self.sidebar_section {
                    SidebarSection::Projects => {
                        let len = self.data.projects.len();
                        if len > 0 {
                            let i = self.data.sidebar_state.selected().unwrap_or(0);
                            self.data.sidebar_state.select(Some((i + 1).min(len - 1)));
                        }
                    }
                    SidebarSection::Models => {
                        let len = self.data.model_pairs.len();
                        if len > 0 {
                            let i = self.data.model_state.selected().unwrap_or(0);
                            self.data.model_state.select(Some((i + 1).min(len - 1)));
                        }
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match self.sidebar_section {
                    SidebarSection::Projects => {
                        let i = self.data.sidebar_state.selected().unwrap_or(0);
                        self.data.sidebar_state.select(Some(i.saturating_sub(1)));
                    }
                    SidebarSection::Models => {
                        let i = self.data.model_state.selected().unwrap_or(0);
                        self.data.model_state.select(Some(i.saturating_sub(1)));
                    }
                }
            }
            KeyCode::Enter => {
                match self.sidebar_section {
                    SidebarSection::Projects => {
                        if let Some(idx) = self.data.sidebar_state.selected() {
                            if Some(idx) != self.data.current_project {
                                self.data.current_project = Some(idx);
                                self.data.graphs.clear();
                                self.data.status = "loading…".to_string();
                                self.data.refresh_side_data().await;
                                self.data.status = "ready".to_string();
                                self.graph.reset();
                            }
                        }
                    }
                    SidebarSection::Models => {
                        if let Some(idx) = self.data.model_state.selected() {
                            if let Some((pid, mid, _)) = self.data.model_pairs.get(idx) {
                                self.data.model_pick = Some((pid.clone(), mid.clone()));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn update_sidebar_selection(&mut self) {
        match self.sidebar_section {
            SidebarSection::Projects => {
                if self.data.sidebar_state.selected().is_none() {
                    self.data.sidebar_state.select(self.data.current_project);
                }
            }
            SidebarSection::Models => {
                if self.data.model_state.selected().is_none() {
                    self.data.model_state.select(Some(0));
                }
            }
        }
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        self.draw_tabbar(f, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(0)])
            .split(chunks[1]);
        self.draw_sidebar(f, body[0]);

        match self.active_view {
            0 => self.chat.render(f, body[1], &self.data),
            1 => self.graph.render(f, body[1], &self.data),
            2 => self.deploy.render(f, body[1], &self.data),
            3 => self.agents.render(f, body[1], &self.data),
            4 => self.artifacts.render(f, body[1], &self.data),
            5 => self.skills.render(f, body[1], &self.data),
            6 => self.admin.render(f, body[1], &self.data),
            _ => {}
        }

        self.draw_statusbar(f, chunks[2]);

        if self.show_help {
            self.draw_help(f, area);
        }
    }

    fn draw_tabbar(&self, f: &mut ratatui::Frame, area: Rect) {
        let titles: Vec<Span> = VIEW_TITLES
            .iter()
            .enumerate()
            .flat_map(|(i, t)| {
                let marker = if i == self.active_view { "▶ " } else { "  " };
                let style = if i == self.active_view {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                vec![
                    Span::styled(marker.to_string(), style),
                    Span::styled(format!("{}. {}  ", i + 1, t), style),
                ]
            })
            .collect();
        let para = Paragraph::new(Line::from(titles))
            .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(para, area);
    }

    fn draw_sidebar(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(10)])
            .split(area);

        let projects_focus =
            self.sidebar_focused && self.sidebar_section == SidebarSection::Projects;

        let mut items: Vec<ListItem> = Vec::new();
        items.push(ListItem::new(Line::from(Span::styled(
            " PROJECTS",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))));
        for (i, p) in self.data.projects.iter().enumerate() {
            let marker = if Some(i) == self.data.current_project {
                "▸ "
            } else {
                "  "
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(marker.to_string(), Style::default().fg(Color::Cyan)),
                Span::styled(p.name.clone(), Style::default().fg(Color::Reset)),
            ])));
        }
        items.push(ListItem::new(Line::from(Span::styled(
            "─ repos ─",
            Style::default().fg(Color::DarkGray),
        ))));
        for r in &self.data.repositories {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::DarkGray)),
                Span::styled(r.name.clone(), Style::default().fg(Color::Reset)),
            ])));
            for v in r.versions.iter().take(3) {
                items.push(ListItem::new(Line::from(vec![
                    Span::styled("    ◦ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(v.clone(), Style::default().fg(Color::Reset)),
                ])));
            }
        }

        let border_style = if projects_focus {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .title(" Harvest ")
                    .border_style(border_style),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, chunks[0], &mut self.data.sidebar_state);

        let models_focus =
            self.sidebar_focused && self.sidebar_section == SidebarSection::Models;

        let mut model_lines: Vec<Line> = vec![Line::from(Span::styled(
            " MODELS",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))];
        for p in self.data.providers.iter().take(4) {
            let label = if p.name.is_empty() {
                p.kind.clone()
            } else {
                p.name.clone()
            };
            let active = self
                .data
                .model_pick
                .as_ref()
                .map(|(id, _)| id == &p.id)
                .unwrap_or(false);
            let marker = if active { "◉" } else { "•" };
            model_lines.push(Line::from(Span::styled(
                format!(" {marker} {label}"),
                Style::default().fg(if active { Color::Yellow } else { Color::Reset }),
            )));
            for m in p.models.iter().take(2) {
                let mlabel = m.display_name.clone().unwrap_or_else(|| m.id.clone());
                model_lines.push(Line::from(Span::styled(
                    format!("     {mlabel}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        let model_border = if models_focus {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let para = Paragraph::new(model_lines).block(
            Block::default()
                .borders(Borders::RIGHT | Borders::TOP)
                .title("")
                .border_style(model_border),
        );
        f.render_widget(para, chunks[1]);
    }

    fn draw_statusbar(&self, f: &mut ratatui::Frame, area: Rect) {
        let sidebar_hint = if self.sidebar_focused {
            " Ctrl-P leave sidebar  j/k move  Enter select "
        } else {
            " q quit  Tab switch  1-7 view  ? help  Ctrl-P sidebar  Ctrl-R refresh "
        };
        let line = Line::from(vec![
            Span::styled(sidebar_hint, Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(self.data.status_line(), Style::default().fg(Color::Yellow)),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn draw_help(&self, f: &mut ratatui::Frame, area: Rect) {
        let help = vec![
            "Harvest TUI — keymap",
            "",
            "  1-7      jump to view",
            "  Tab      next view",
            "  Shift-Tab prev view",
            "  q        quit",
            "  ? / F1   toggle this help",
            "  Ctrl-P   focus sidebar (projects/models)",
            "  Ctrl-R   refresh data",
            "",
            "Sidebar:",
            "  j/k      move selection",
            "  Tab      toggle projects/models",
            "  Enter    select project or model",
            "  Esc      leave sidebar",
            "",
            "Chat:",
            "  Alt-Enter or Ctrl-S  send query",
            "  ↑/↓      input history",
            "  PgUp/PgDn scroll transcript",
            "  Enter    send / open citation",
            "  y/n      approve / deny confirm-action",
            "  1-9      answer question card",
            "  Ctrl-H   conversation history",
            "",
            "Graph:",
            "  j/k      move symbol",
            "  Enter    view source",
            "  /        search",
            "  g        ascii adjacency",
            "  Tab      focus detail panel",
            "  Ctrl-R   reload graph",
            "",
            "Agents:",
            "  x        execute command popup",
            "  a        add agent",
            "  r        restart agent",
            "  d        delete agent",
            "  f        port forwards",
            "  Enter    select agent",
            "",
            "Artifacts:",
            "  j/k      move",
            "  Enter    open detail",
            "  c        create artifact",
            "  d        download artifact",
            "  D        delete artifact",
            "  r        run on agent (terraform)",
            "",
            "Skills:",
            "  j/k      move",
            "  Enter    open detail",
            "  c        create skill",
            "  e        edit skill",
            "  D        delete skill",
            "",
            "Admin:",
            "  u/g      users / groups",
            "  j/k      move",
            "",
            "Press ? or Esc to close.",
        ];
        let lines: Vec<Line> = help
            .iter()
            .map(|l| Line::from(Span::styled(*l, Style::default().fg(Color::Reset))))
            .collect();
        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help ")
                .style(Style::default().bg(Color::Black)),
        );
        let area = centered(area, 65, 80);
        f.render_widget(para, area);
    }
}

fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let h = area.height * height_pct / 100;
    let w = area.width * width_pct / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}