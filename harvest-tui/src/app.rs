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
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::api::{Client, ClientConfig, QueryRequest};
use crate::chat::ChatView;

pub enum AppEvent {
    Chat(crate::api::AgentEvent),
    ChatError(String),
}

pub struct AppData {
    pub client: Client,
    pub status: String,
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

        let data = AppData {
            client,
            status: String::from("ready"),
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

        self.chat.handle_key_async(key, &mut self.data).await;
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        self.chat.render(f, chunks[0], &self.data);
        self.draw_statusbar(f, chunks[1]);

        if self.show_help {
            self.draw_help(f, area);
        }
    }

    fn draw_statusbar(&self, f: &mut ratatui::Frame, area: Rect) {
        let line = Line::from(vec![
            Span::styled(
                " q quit  ? help  Alt-Enter send  Ctrl-S send  PgUp/PgDn scroll ",
                Style::default().fg(Color::DarkGray),
            ),
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
