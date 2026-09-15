use std::io::Stdout;
use std::time::Duration;

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
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::api::{Client, ClientConfig, QueryRequest};
use crate::chat::ChatView;

pub enum AppEvent {
    Chat(crate::api::AgentEvent),
    ChatError(String),
    Authenticated(String, String),
    AuthFailed(String),
}

pub struct AppData {
    pub client: Client,
    pub status: String,
    pub authenticated: bool,
    pub user_email: Option<String>,
    pub auth_url: Option<String>,
    pub auth_polling: bool,
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl AppData {
    pub fn spawn_stream(&self, body: QueryRequest) {
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

    pub fn start_tui_login(&mut self) {
        if self.auth_polling {
            return;
        }
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        self.auth_polling = true;
        self.status = "Requesting authorization…".to_string();

        tokio::spawn(async move {
            let body = serde_json::json!({});
            match client.post_json::<_, serde_json::Value>("/auth/tui/login", &body).await {
                Ok(resp) => {
                    let uuid = resp.get("uuid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let auth_url = resp.get("auth_url").and_then(|v| v.as_str()).unwrap_or("").to_string();

                    if uuid.is_empty() {
                        let _ = tx.send(AppEvent::AuthFailed("no uuid returned".to_string()));
                        return;
                    }

                    let _ = tx.send(AppEvent::AuthFailed(format!("__auth_url__{}", auth_url)));

                    let poll_client = client.clone();
                    for _ in 0..150 {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        match poll_client.get_json::<serde_json::Value>(&format!("/auth/tui/poll/{}", uuid)).await {
                            Ok(resp) => {
                                let status = resp.get("status").and_then(|v| v.as_str()).unwrap_or("");
                                match status {
                                    "authorized" => {
                                        let token = resp.get("token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let email = resp.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let _ = tx.send(AppEvent::Authenticated(token, email));
                                        return;
                                    }
                                    "denied" => {
                                        let _ = tx.send(AppEvent::AuthFailed("denied".to_string()));
                                        return;
                                    }
                                    "expired" => {
                                        let _ = tx.send(AppEvent::AuthFailed("expired".to_string()));
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            Err(_) => {}
                        }
                    }
                    let _ = tx.send(AppEvent::AuthFailed("timeout".to_string()));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::AuthFailed(format!("login request failed: {}", e)));
                }
            }
        });
    }
}

pub struct App {
    pub data: AppData,
    pub chat: ChatView,
    pub event_rx: mpsc::UnboundedReceiver<AppEvent>,
    pub should_quit: bool,
    pub show_help: bool,
}

impl App {
    pub async fn new(cfg: ClientConfig) -> Result<Self> {
        let client = Client::new(cfg)?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let authenticated = client.token.is_some();
        let status = if authenticated {
            "ready".to_string()
        } else {
            "not authenticated — type /login".to_string()
        };

        let data = AppData {
            client,
            status,
            authenticated,
            user_email: None,
            auth_url: None,
            auth_polling: false,
            event_tx,
        };

        Ok(Self {
            data,
            chat: ChatView::new(),
            event_rx,
            should_quit: false,
            show_help: false,
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
            AppEvent::Authenticated(token, email) => {
                self.data.client = self.data.client.rebuild_with_token(token);
                self.data.authenticated = true;
                self.data.user_email = Some(email.clone());
                self.data.auth_url = None;
                self.data.auth_polling = false;
                self.data.status = format!("authenticated as {}", email);
            }
            AppEvent::AuthFailed(msg) => {
                if msg.starts_with("__auth_url__") {
                    let url = msg["__auth_url__".len()..].to_string();
                    self.data.auth_url = Some(url);
                    self.data.status = "waiting for authorization…".to_string();
                } else {
                    self.data.auth_url = None;
                    self.data.auth_polling = false;
                    self.data.status = format!("auth failed: {}", msg);
                }
            }
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
        {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::F(1) || (key.code == KeyCode::Char('?') && key.modifiers == KeyModifiers::NONE) {
            self.show_help = true;
            return;
        }

        if !self.data.authenticated {
            self.chat.handle_key_async(key, &mut self.data).await;
            return;
        }

        self.chat.handle_key_async(key, &mut self.data).await;
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        if self.data.authenticated {
            self.chat.render(f, chunks[0], &self.data);
        } else {
            self.draw_unauthenticated(f, chunks[0]);
        }

        self.draw_statusbar(f, chunks[1]);

        if self.show_help {
            self.draw_help(f, area);
        }
    }

    fn draw_unauthenticated(&self, f: &mut ratatui::Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(Span::styled(
            " Not authenticated",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Type /login to authenticate via the Harvest Web UI.",
            Style::default().fg(Color::Reset),
        )));
        lines.push(Line::from(Span::styled(
            " Or set HARVEST_EMAIL + HARVEST_PASSWORD environment variables.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        if let Some(url) = &self.data.auth_url {
            lines.push(Line::from(Span::styled(
                " Open this link in your browser to authenticate:",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {}", url),
                Style::default().fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Waiting for authorization…",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let input_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area);

        let para = Paragraph::new(lines);
        f.render_widget(para, input_area[0]);

        let input_text = self.chat.get_input_for_auth();
        let input_para = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(input_text.clone()),
            Span::styled(" ", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Type /login to authenticate "),
        );
        f.render_widget(input_para, input_area[1]);
    }

    fn draw_statusbar(&self, f: &mut ratatui::Frame, area: Rect) {
        let hint = if self.data.authenticated {
            " q quit  ? help  Alt-Enter send  Ctrl-S send  PgUp/PgDn scroll "
        } else {
            " q quit  ? help  /login to authenticate "
        };
        let line = Line::from(vec![
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(
                format!(" {} ", self.data.status),
                Style::default().fg(Color::Yellow),
            ),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn draw_help(&self, f: &mut ratatui::Frame, area: Rect) {
        let help = vec![
            "Harvest TUI — keymap",
            "",
            "  q               quit",
            "  ? / F1          toggle this help",
            "  /login          authenticate via Web UI",
            "",
            "Chat:",
            "  Alt-Enter       send query",
            "  Ctrl-S          send query",
            "  Enter           send query",
            "  ↑/↓             input history",
            "  PgUp/PgDn       scroll transcript",
            "  Ctrl-J/Ctrl-K   scroll 3 lines",
            "  Tab             step mode (while streaming)",
            "",
            "Step mode:",
            "  j/k             select step",
            "  Enter           expand/collapse detail",
            "  Esc             exit step mode",
            "",
            "  y/n             approve / deny confirm-action",
            "  1-9             answer question card",
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
        let area = centered(area, 55, 70);
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
