use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::api::{GraphData, GraphEdge, GraphNode, SymbolSource};
use crate::app::AppData;

pub struct GraphView {
    pub graph: Option<GraphData>,
    symbols: Vec<(String, GraphNode)>,
    tree: Vec<TreeEntry>,
    tree_state: ListState,
    #[allow(dead_code)]
    selected: Option<usize>,
    pub search: String,
    pub searching: bool,
    pub source_pop: Option<(GraphNode, Option<SymbolSource>)>,
    ascii_view: bool,
    file_order: Vec<String>,
    detail_focus: bool,
    detail_state: ListState,
    source_scroll: usize,
}

struct TreeEntry {
    kind: TreeKind,
    label: String,
    node_id: Option<String>,
    depth: usize,
    expanded: bool,
}

enum TreeKind {
    File,
    Symbol,
}

impl GraphView {
    pub fn new() -> Self {
        let mut s = Self {
            graph: None,
            symbols: Vec::new(),
            tree: Vec::new(),
            tree_state: ListState::default(),
            selected: None,
            search: String::new(),
            searching: false,
            source_pop: None,
            ascii_view: false,
            file_order: Vec::new(),
            detail_focus: false,
            detail_state: ListState::default(),
            source_scroll: 0,
        };
        s.tree_state.select(Some(0));
        s
    }

    pub fn reset(&mut self) {
        self.graph = None;
        self.symbols.clear();
        self.tree.clear();
        self.file_order.clear();
        self.source_pop = None;
        self.detail_focus = false;
        self.source_scroll = 0;
    }

    async fn ensure_graph(&mut self, app: &AppData) {
        if self.graph.is_some() {
            return;
        }
        let Some((repo, version)) = app.current_repo_and_version() else {
            return;
        };
        let path = format!("/graph/{repo}/{version}");
        if let Ok(data) = app.client.get_json::<GraphData>(&path).await {
            self.load_graph(data);
        }
    }

    fn load_graph(&mut self, data: GraphData) {
        let mut by_file: HashMap<String, Vec<GraphNode>> = HashMap::new();
        let mut file_order: Vec<String> = Vec::new();
        for n in &data.nodes {
            if !by_file.contains_key(&n.file) {
                file_order.push(n.file.clone());
            }
            by_file.entry(n.file.clone()).or_default().push(n.clone());
        }
        file_order.sort();

        let mut symbols: Vec<(String, GraphNode)> = Vec::new();
        let mut tree: Vec<TreeEntry> = Vec::new();
        for file in &file_order {
            tree.push(TreeEntry {
                kind: TreeKind::File,
                label: file.clone(),
                node_id: None,
                depth: 0,
                expanded: false,
            });
            if let Some(nodes) = by_file.get(file) {
                let mut sorted = nodes.clone();
                sorted.sort_by_key(|n| n.start_line);
                for n in sorted {
                    symbols.push((n.id.clone(), n.clone()));
                    tree.push(TreeEntry {
                        kind: TreeKind::Symbol,
                        label: format!("{}() [{}]", n.name, n.kind),
                        node_id: Some(n.id.clone()),
                        depth: 1,
                        expanded: false,
                    });
                }
            }
        }
        self.file_order = file_order;
        self.symbols = symbols;
        self.tree = tree;
        self.graph = Some(data);
    }

    fn current_node(&self) -> Option<&GraphNode> {
        let idx = self.tree_state.selected()?;
        let entry = self.tree.get(idx)?;
        let id = entry.node_id.as_ref()?;
        self.symbols.iter().find(|(nid, _)| nid == id).map(|(_, n)| n)
    }

    fn neighbors(&self, node_id: &str, dir: NeighborDir) -> Vec<&GraphEdge> {
        let Some(g) = &self.graph else {
            return Vec::new();
        };
        g.edges
            .iter()
            .filter(|e| match dir {
                NeighborDir::Out => e.source == node_id,
                NeighborDir::In => e.target == node_id,
            })
            .collect()
    }

    fn node_name(&self, id: &str) -> &str {
        self.symbols
            .iter()
            .find(|(nid, _)| nid == id)
            .map(|(_, n)| n.name.as_str())
            .unwrap_or("?")
    }

    fn node_by_id(&self, id: &str) -> Option<&GraphNode> {
        self.symbols
            .iter()
            .find(|(nid, _)| nid == id)
            .map(|(_, n)| n)
    }

    async fn load_source(&self, app: &AppData, node: &GraphNode) -> Option<SymbolSource> {
        let (repo, version) = app.current_repo_and_version()?;
        let path = format!(
            "/graph/{repo}/{version}/source?file={}&name={}",
            urlencode(&node.file),
            urlencode(&node.name)
        );
        app.client.get_json::<SymbolSource>(&path).await.ok()
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, _app: &AppData) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let total = self.graph.as_ref().map(|g| g.nodes.len()).unwrap_or(0);
        let truncated = self.graph.as_ref().map(|g| g.truncated).unwrap_or(false);
        let title = if self.searching {
            format!(" search: {}_ ", self.search)
        } else {
            let mut t = format!(" Graph  [ {total} symbols ]");
            if truncated {
                t.push_str(" (truncated)");
            }
            t
        };
        let header = Paragraph::new(Line::from(Span::styled(
            title,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Min(0)])
            .split(chunks[1]);

        let mut items: Vec<ListItem> = Vec::new();
        let filter = self.search.to_lowercase();
        for entry in &self.tree {
            if !filter.is_empty() && !entry.label.to_lowercase().contains(&filter) {
                continue;
            }
            let indent = "  ".repeat(entry.depth);
            let (marker, color) = match entry.kind {
                TreeKind::File => {
                    if entry.expanded {
                        ("▾", Color::Cyan)
                    } else {
                        ("▸", Color::Cyan)
                    }
                }
                TreeKind::Symbol => ("•", Color::Reset),
            };
            items.push(ListItem::new(Line::from(vec![
                Span::raw(indent.clone()),
                Span::styled(format!("{marker} "), Style::default().fg(color)),
                Span::styled(entry.label.clone(), Style::default().fg(color)),
            ])));
        }
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .title(" Symbols "),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, body[0], &mut self.tree_state);

        let detail = if self.ascii_view {
            self.render_ascii()
        } else {
            self.render_detail()
        };
        let para = Paragraph::new(detail).block(
            Block::default()
                .borders(Borders::LEFT)
                .title(" Details "),
        );
        f.render_widget(para, body[1]);

        if let Some((node, sym)) = &self.source_pop {
            let pop = centered(area, 75, 70);
            let content = sym
                .as_ref()
                .and_then(|s| s.source.clone())
                .unwrap_or_else(|| "(no source available)".to_string());
            let start = sym.as_ref().map(|s| s.start_line).unwrap_or(node.start_line);
            let lines: Vec<Line> = content
                .lines()
                .enumerate()
                .map(|(i, l)| {
                    Line::from(vec![
                        Span::styled(
                            format!("{:>4} ", start + i as i64),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(l.to_string(), Style::default().fg(Color::Green)),
                    ])
                })
                .collect();
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {}:{} ", node.file, node.start_line)),
                ),
                pop,
            );
        }
    }

    fn render_detail(&self) -> Vec<Line<'_>> {
        let mut lines: Vec<Line> = Vec::new();
        let Some(node) = self.current_node() else {
            lines.push(Line::from(Span::styled(
                " select a symbol on the left",
                Style::default().fg(Color::DarkGray),
            )));
            return lines;
        };
        lines.push(Line::from(vec![
            Span::styled(" name: ", Style::default().fg(Color::Cyan)),
            Span::styled(node.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(" kind: ", Style::default().fg(Color::Cyan)),
            Span::styled(node.kind.clone(), Style::default()),
        ]));
        lines.push(Line::from(vec![
            Span::styled(" file: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{}:{}", node.file, node.start_line),
                Style::default(),
            ),
        ]));
        if let Some(sig) = &node.signature {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " signature:",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
            for l in sig.lines() {
                lines.push(Line::from(Span::styled(
                    format!("   {l}"),
                    Style::default().fg(Color::Green),
                )));
            }
        }
        lines.push(Line::from(""));

        let callers: Vec<_> = self.neighbors(&node.id, NeighborDir::In)
            .into_iter()
            .filter(|e| e.relation == "calls")
            .collect();
        if !callers.is_empty() {
            lines.push(Line::from(Span::styled(
                " CALLED BY",
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )));
            for e in callers {
                let name = self.node_name(&e.source).to_string();
                let file = self
                    .node_by_id(&e.source)
                    .map(|n| format!("{}:{}", n.file, n.start_line))
                    .unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled("   ▸ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(name, Style::default().fg(Color::Reset)),
                    Span::styled(
                        format!("   ({file})"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            lines.push(Line::from(""));
        }

        let callees: Vec<_> = self.neighbors(&node.id, NeighborDir::Out)
            .into_iter()
            .filter(|e| e.relation == "calls")
            .collect();
        if !callees.is_empty() {
            lines.push(Line::from(Span::styled(
                " CALLS",
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )));
            for e in callees {
                let name = self.node_name(&e.target).to_string();
                let file = self
                    .node_by_id(&e.target)
                    .map(|n| format!("{}:{}", n.file, n.start_line))
                    .unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled("   ▸ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(name, Style::default().fg(Color::Reset)),
                    Span::styled(
                        format!("   ({file})"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            lines.push(Line::from(""));
        }

        let relations: Vec<_> = self
            .neighbors(&node.id, NeighborDir::Out)
            .into_iter()
            .filter(|e| e.relation != "calls")
            .collect();
        if !relations.is_empty() {
            lines.push(Line::from(Span::styled(
                " RELATIONSHIPS",
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )));
            for e in relations {
                let name = self.node_name(&e.target).to_string();
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("   {} ", e.relation),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(name, Style::default().fg(Color::Reset)),
                ]));
            }
        }

        lines.push(Line::from(""));
        let footer = if self.detail_focus {
            " Tab tree  Enter jump  j/k move"
        } else {
            " Enter source  g ascii-graph  / search  Tab detail"
        };
        lines.push(Line::from(Span::styled(
            footer,
            Style::default().fg(Color::DarkGray),
        )));
        lines
    }

    fn render_ascii(&self) -> Vec<Line<'_>> {
        let mut lines: Vec<Line> = Vec::new();
        let Some(node) = self.current_node() else {
            lines.push(Line::from("(no symbol)"));
            return lines;
        };

        let center = &node.name;
        let callers: Vec<String> = self
            .neighbors(&node.id, NeighborDir::In)
            .into_iter()
            .filter(|e| e.relation == "calls")
            .map(|e| self.node_name(&e.source).to_string())
            .collect();
        let callees: Vec<String> = self
            .neighbors(&node.id, NeighborDir::Out)
            .into_iter()
            .filter(|e| e.relation == "calls")
            .map(|e| self.node_name(&e.target).to_string())
            .collect();

        lines.push(Line::from(Span::styled(
            " 2-hop call graph",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        for caller in &callers {
            lines.push(Line::from(vec![
                Span::styled("       ┌─────────┐\n", Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("       │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    pad(caller, 9),
                    Style::default().fg(Color::Blue),
                ),
                Span::styled(" │", Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("       └────┬────┘\n", Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("            │ calls\n", Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("            ▼\n", Style::default().fg(Color::DarkGray)),
            ]));
        }

        lines.push(Line::from(vec![
            Span::styled("       ┌─────────┐    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if callees.is_empty() {
                    String::new()
                } else {
                    "┌─────────────┐".to_string()
                },
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("       │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                pad(center, 9),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" │", Style::default().fg(Color::DarkGray)),
            if callees.is_empty() {
                Span::raw("")
            } else {
                Span::styled(
                    "───▶ ",
                    Style::default().fg(Color::DarkGray),
                )
            },
            if let Some(first) = callees.first() {
                Span::styled(
                    pad(first, 13),
                    Style::default().fg(Color::Green),
                )
            } else {
                Span::raw("")
            },
        ]));
        lines.push(Line::from(vec![
            Span::styled("       └─────────┘    ", Style::default().fg(Color::DarkGray)),
            if callees.len() > 1 {
                Span::styled("└──────┬──────┘", Style::default().fg(Color::DarkGray))
            } else if callees.is_empty() {
                Span::raw("")
            } else {
                Span::styled("└─────────────┘", Style::default().fg(Color::DarkGray))
            },
        ]));
        if callees.len() > 1 {
            for c in callees.iter().skip(1) {
                lines.push(Line::from(vec![
                    Span::styled("                       ", Style::default().fg(Color::DarkGray)),
                    Span::styled("▶ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(c.clone(), Style::default().fg(Color::Green)),
                ]));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " g back to detail  Enter source",
            Style::default().fg(Color::DarkGray),
        )));
        lines
    }

    pub async fn handle_key_async(&mut self, key: KeyEvent, app: &mut AppData) {
        self.ensure_graph(app).await;
        if self.source_pop.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.source_pop = None;
                    self.source_scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.source_scroll = self.source_scroll.saturating_add(1);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.source_scroll = self.source_scroll.saturating_sub(1);
                }
                _ => {}
            }
            return;
        }

        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.search.clear();
                }
                KeyCode::Enter => {
                    self.searching = false;
                }
                KeyCode::Backspace => {
                    self.search.pop();
                }
                KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                    self.search.push(c);
                }
                _ => {}
            }
            return;
        }

        if key.code == KeyCode::Tab {
            self.detail_focus = !self.detail_focus;
            return;
        }

        if self.detail_focus && !self.ascii_view {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    let len = self.relation_count();
                    if len > 0 {
                        let i = self.detail_state.selected().unwrap_or(0);
                        self.detail_state.select(Some((i + 1).min(len - 1)));
                    }
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let i = self.detail_state.selected().unwrap_or(0);
                    self.detail_state.select(Some(i.saturating_sub(1)));
                    return;
                }
                KeyCode::Enter => {
                    if let Some(target_id) = self.relation_target() {
                        if let Some(idx) = self
                            .tree
                            .iter()
                            .position(|e| e.node_id.as_deref() == Some(target_id.as_str()))
                        {
                            self.tree_state.select(Some(idx));
                            self.detail_focus = false;
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.graph = None;
                return;
            }
            KeyCode::Char('/') => {
                self.searching = true;
                self.search.clear();
                return;
            }
            KeyCode::Char('g') => {
                self.ascii_view = !self.ascii_view;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.visible_count();
                if len > 0 {
                    let i = self.tree_state.selected().unwrap_or(0);
                    self.tree_state.select(Some((i + 1).min(len - 1)));
                }
                return;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.tree_state.selected().unwrap_or(0);
                self.tree_state.select(Some(i.saturating_sub(1)));
                return;
            }
            KeyCode::PageDown => {
                let len = self.visible_count();
                let i = self.tree_state.selected().unwrap_or(0);
                self.tree_state.select(Some((i + 10).min(len.saturating_sub(1))));
                return;
            }
            KeyCode::PageUp => {
                let i = self.tree_state.selected().unwrap_or(0);
                self.tree_state.select(Some(i.saturating_sub(10)));
                return;
            }
            KeyCode::Enter => {
                if let Some(node) = self.current_node().cloned() {
                    let sym = self.load_source(app, &node).await;
                    self.source_pop = Some((node, sym));
                    self.source_scroll = 0;
                }
                return;
            }
            _ => {}
        }
    }

    fn relation_count(&self) -> usize {
        let Some(node) = self.current_node() else {
            return 0;
        };
        let callers = self.neighbors(&node.id, NeighborDir::In)
            .iter()
            .filter(|e| e.relation == "calls")
            .count();
        let callees = self.neighbors(&node.id, NeighborDir::Out)
            .iter()
            .filter(|e| e.relation == "calls")
            .count();
        let relations = self.neighbors(&node.id, NeighborDir::Out)
            .iter()
            .filter(|e| e.relation != "calls")
            .count();
        callers + callees + relations
    }

    fn relation_target(&self) -> Option<String> {
        let node = self.current_node()?;
        let idx = self.detail_state.selected()?;
        let mut count = 0;
        for e in self.neighbors(&node.id, NeighborDir::In) {
            if e.relation == "calls" {
                if count == idx {
                    return Some(e.source.clone());
                }
                count += 1;
            }
        }
        for e in self.neighbors(&node.id, NeighborDir::Out) {
            if e.relation == "calls" {
                if count == idx {
                    return Some(e.target.clone());
                }
                count += 1;
            }
        }
        for e in self.neighbors(&node.id, NeighborDir::Out) {
            if e.relation != "calls" {
                if count == idx {
                    return Some(e.target.clone());
                }
                count += 1;
            }
        }
        None
    }

    fn visible_count(&self) -> usize {
        let filter = self.search.to_lowercase();
        if filter.is_empty() {
            return self.tree.len();
        }
        self.tree
            .iter()
            .filter(|e| e.label.to_lowercase().contains(&filter))
            .count()
    }
}

enum NeighborDir {
    In,
    Out,
}

fn pad(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        s.chars().take(w).collect()
    } else {
        let mut out = s.to_string();
        out.push_str(&" ".repeat(w - len));
        out
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let h = area.height * height_pct / 100;
    let w = area.width * width_pct / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}
