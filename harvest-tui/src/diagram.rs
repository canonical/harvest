use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug)]
struct DiagNode {
    id: String,
    label: String,
    shape: NodeShape,
    subgraph: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum NodeShape {
    Box,
    Round,
    Diamond,
    Circle,
    Plain,
}

#[derive(Clone, Debug)]
struct DiagEdge {
    from: String,
    to: String,
    label: Option<String>,
    dotted: bool,
}

struct ParsedDiagram {
    direction: Direction,
    nodes: BTreeMap<String, DiagNode>,
    edges: Vec<DiagEdge>,
    node_order: Vec<String>,
    subgraphs: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Direction {
    TD,
    LR,
}

pub fn render_diagram(source: &str, lang: &str, width: usize) -> Vec<Line<'static>> {
    match lang {
        "mermaid" => render_mermaid(source, width),
        "harvest-graph" => render_harvest_graph(source, width),
        _ => render_code(source, lang, width),
    }
}

fn render_code(source: &str, lang: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !lang.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" {} ", lang),
            Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));
    }
    let w = width.saturating_sub(2).max(20);
    for l in source.lines().take(30) {
        let truncated = if l.len() > w { format!("{}…", &l[..w.saturating_sub(1)]) } else { l.to_string() };
        lines.push(Line::from(Span::styled(truncated, Style::default().fg(Color::Green))));
    }
    lines.push(Line::from(""));
    lines
}

fn render_mermaid(source: &str, width: usize) -> Vec<Line<'static>> {
    let parsed = match parse_mermaid(source) {
        Some(p) => p,
        None => return render_code(source, "mermaid", width),
    };
    render_flowchart(&parsed, width)
}

fn parse_mermaid(source: &str) -> Option<ParsedDiagram> {
    let mut lines = source.lines();
    let first = lines.next()?.trim();
    let direction = parse_direction(first)?;
    let direction = direction?;

    let mut nodes: BTreeMap<String, DiagNode> = BTreeMap::new();
    let mut edges: Vec<DiagEdge> = Vec::new();
    let mut node_order: Vec<String> = Vec::new();
    let mut subgraphs: Vec<(String, Vec<String>)> = Vec::new();
    let mut current_subgraph: Option<String> = None;

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("%%") || line.starts_with("classDef") || line.starts_with("class ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("subgraph ") {
            let name = rest.trim().trim_matches('"').to_string();
            current_subgraph = Some(name.clone());
            subgraphs.push((name, Vec::new()));
            continue;
        }
        if line == "end" {
            current_subgraph = None;
            continue;
        }

        let (parsed_nodes, parsed_edges) = parse_line(line);
        for n in &parsed_nodes {
            if !nodes.contains_key(&n.id) {
                let mut node = n.clone();
                node.subgraph = current_subgraph.clone();
                nodes.insert(n.id.clone(), node.clone());
                node_order.push(n.id.clone());
                if let Some(sg) = &current_subgraph {
                    if let Some(entry) = subgraphs.iter_mut().find(|(n, _)| n == sg) {
                        entry.1.push(n.id.clone());
                    }
                }
            } else {
                if let Some(existing) = nodes.get_mut(&n.id) {
                    if existing.label == existing.id {
                        existing.label = n.label.clone();
                        existing.shape = n.shape;
                    }
                    if existing.subgraph.is_none() {
                        existing.subgraph = current_subgraph.clone();
                    }
                }
            }
        }
        for e in &parsed_edges {
            for id in [&e.from, &e.to] {
                if !nodes.contains_key(id) {
                    let node = DiagNode {
                        id: id.clone(),
                        label: id.clone(),
                        shape: NodeShape::Plain,
                        subgraph: current_subgraph.clone(),
                    };
                    nodes.insert(id.clone(), node);
                    node_order.push(id.clone());
                }
            }
            edges.push(e.clone());
        }
    }

    Some(ParsedDiagram {
        direction,
        nodes,
        edges,
        node_order,
        subgraphs,
    })
}

fn parse_direction(line: &str) -> Option<Option<Direction>> {
    let lower = line.to_lowercase();
    if lower.starts_with("graph ") || lower.starts_with("flowchart ") {
        let dir = match lower.split_whitespace().nth(1) {
            Some("td") | Some("tb") | None => Direction::TD,
            Some("lr") => Direction::LR,
            Some("rl") => Direction::LR,
            Some("bt") => Direction::TD,
            _ => Direction::TD,
        };
        Some(Some(dir))
    } else {
        Some(None)
    }
}

fn parse_line(line: &str) -> (Vec<DiagNode>, Vec<DiagEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let (parts, edge_labels) = tokenize_line(line);
    if parts.is_empty() {
        return (nodes, edges);
    }

    if parts.len() == 1 && edge_labels.is_empty() {
        if let Some(n) = parse_node_token(&parts[0]) {
            nodes.push(n);
        }
        return (nodes, edges);
    }

    let mut prev_id: Option<String> = None;
    let mut label_idx = 0;
    let mut i = 0;
    while i < parts.len() {
        let part = &parts[i];
        if is_edge_token(part) {
            let dotted = part.contains('.') || part.contains('-');
            let label = edge_labels.get(label_idx).cloned().flatten();
            label_idx += 1;

            if let (Some(from), Some(next_node)) = (&prev_id, parts.get(i + 1)) {
                let to_node = parse_node_token(next_node);
                let to_id = to_node.as_ref().map(|n| n.id.clone()).unwrap_or_else(|| next_node.clone());
                edges.push(DiagEdge {
                    from: from.clone(),
                    to: to_id.clone(),
                    label,
                    dotted,
                });
                if let Some(n) = to_node {
                    nodes.push(n);
                }
                prev_id = Some(to_id);
            }
            i += 2;
        } else {
            if let Some(n) = parse_node_token(part) {
                if prev_id.is_none() {
                    nodes.push(n.clone());
                }
                prev_id = Some(n.id);
            } else {
                prev_id = Some(part.clone());
            }
            i += 1;
        }
    }

    (nodes, edges)
}

fn tokenize_line(line: &str) -> (Vec<String>, Vec<Option<String>>) {
    let mut parts: Vec<String> = Vec::new();
    let mut labels: Vec<Option<String>> = Vec::new();

    let mut remaining = line.trim().to_string();

    loop {
        let trimmed = remaining.trim();
        if trimmed.is_empty() {
            break;
        }

        if let Some((edge, label, rest)) = extract_edge_token(trimmed) {
            parts.push(edge);
            labels.push(label);
            remaining = rest.to_string();
            continue;
        }

        if let Some((node, rest)) = extract_node_token(trimmed) {
            parts.push(node);
            remaining = rest.to_string();
            continue;
        }

        let word = trimmed.split_whitespace().next().unwrap_or("");
        if word.is_empty() {
            break;
        }
        if word.starts_with('-') || word.starts_with('=') {
            parts.push("-->".to_string());
            labels.push(None);
            let advance = word.len();
            remaining = trimmed[advance..].to_string();
            continue;
        }
        parts.push(word.to_string());
        remaining = trimmed[word.len()..].to_string();
    }

    (parts, labels)
}

fn extract_node_token(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let first_char = s.chars().next()?;
    if !first_char.is_alphanumeric() && first_char != '_' {
        return None;
    }
    let id_end = s.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')?;
    let id = &s[..id_end];
    if id.is_empty() {
        return None;
    }
    let rest = &s[id_end..];
    let rest_trimmed = rest.trim_start();

    for (open, close, shape) in [
        ("((", "))", NodeShape::Circle),
        ("[", "]", NodeShape::Box),
        ("{", "}", NodeShape::Diamond),
        ("(", ")", NodeShape::Round),
    ] {
        if rest_trimmed.starts_with(open) {
            if let Some(end) = find_matching(rest_trimmed, open, close) {
                let inner = &rest_trimmed[open.len()..end];
                let label = inner.trim_matches('"').to_string();
                let token = format!("{}{}{}{}", id, open, label, close);
                let after = &rest_trimmed[end + close.len()..];
                return Some((token, after));
            }
        }
    }

    let after = rest;
    Some((id.to_string(), after))
}

fn find_matching(s: &str, open: &str, close: &str) -> Option<usize> {
    let mut depth = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    let open_bytes = open.as_bytes();
    let close_bytes = close.as_bytes();
    while i < bytes.len() {
        if i + open_bytes.len() <= bytes.len() && &bytes[i..i + open_bytes.len()] == open_bytes {
            depth += 1;
            i += open_bytes.len();
            continue;
        }
        if i + close_bytes.len() <= bytes.len() && &bytes[i..i + close_bytes.len()] == close_bytes {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            i += close_bytes.len();
            continue;
        }
        i += 1;
    }
    None
}

fn extract_edge_token(s: &str) -> Option<(String, Option<String>, &str)> {
    let s = s.trim_start();

    if s.starts_with("-- ") && !s.starts_with("--- ") {
        for end_pat in ["-->", "---", "==>", "-.->"] {
            if let Some(pos) = s.find(end_pat) {
                if pos > 3 {
                    let label = s[3..pos].trim().to_string();
                    let after = s[pos + end_pat.len()..].trim();
                    return Some((end_pat.to_string(), Some(label), after));
                }
            }
        }
    }

    let patterns: &[(&str, bool)] = &[
        ("-.->", true),
        ("==>", false),
        ("-->", false),
        ("---", false),
        ("===", false),
        ("-.-", true),
    ];

    for &(pat, _dotted) in patterns {
        if s.starts_with(pat) {
            let after = s[pat.len()..].trim();

            let label = if after.starts_with('|') {
                if let Some(end) = after[1..].find('|') {
                    let l = after[1..1 + end].trim().to_string();
                    Some(l)
                } else {
                    None
                }
            } else {
                None
            };

            let after_str = if after.starts_with('|') {
                if let Some(end) = after[1..].find('|') {
                    &after[2 + end..]
                } else {
                    after
                }
            } else {
                after
            };

            return Some((pat.to_string(), label, after_str));
        }
    }

    None
}

fn is_edge_token(s: &str) -> bool {
    s.contains("-->") || s.contains("==>") || s.contains("-.->") || s.contains("---") || s.contains("===") || s.contains("-.-")
}

fn parse_node_token(token: &str) -> Option<DiagNode> {
    let token = token.trim();
    let id_end = token.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-').unwrap_or(token.len());
    let id = token[..id_end].to_string();
    if id.is_empty() {
        return None;
    }
    let rest = &token[id_end..];

    for (open, close, shape) in [
        ("((", "))", NodeShape::Circle),
        ("[", "]", NodeShape::Box),
        ("{", "}", NodeShape::Diamond),
        ("(", ")", NodeShape::Round),
    ] {
        if let Some(start) = rest.find(open) {
            if let Some(end) = find_matching(&rest[start..], open, close) {
                let inner = &rest[start + open.len()..start + end];
                let label = inner.trim_matches('"').to_string();
                return Some(DiagNode {
                    id,
                    label,
                    shape,
                    subgraph: None,
                });
            }
        }
    }

    Some(DiagNode {
        id: id.clone(),
        label: id,
        shape: NodeShape::Plain,
        subgraph: None,
    })
}

fn render_flowchart(diag: &ParsedDiagram, width: usize) -> Vec<Line<'static>> {
    match diag.direction {
        Direction::TD => render_flowchart_td(diag, width),
        Direction::LR => render_flowchart_lr(diag, width),
    }
}

fn compute_layers(diag: &ParsedDiagram) -> Vec<Vec<String>> {
    let incoming: HashMap<String, usize> = diag.edges.iter().fold(HashMap::new(), |mut acc, e| {
        *acc.entry(e.to.clone()).or_insert(0) += 1;
        acc
    });

    let roots: Vec<String> = diag.node_order.iter()
        .filter(|id| !incoming.contains_key(*id))
        .cloned()
        .collect();

    let mut layer: HashMap<String, usize> = HashMap::new();
    for r in &roots {
        layer.insert(r.clone(), 0);
    }

    let mut changed = true;
    while changed {
        changed = false;
        for e in &diag.edges {
            if let Some(&from_layer) = layer.get(&e.from) {
                let new_layer = from_layer + 1;
                if layer.get(&e.to).map_or(true, |&l| l < new_layer) {
                    layer.insert(e.to.clone(), new_layer);
                    changed = true;
                }
            }
        }
    }

    for id in diag.node_order.iter() {
        layer.entry(id.clone()).or_insert(0);
    }

    let max_layer = layer.values().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<String>> = vec![Vec::new(); max_layer + 1];
    for id in &diag.node_order {
        if let Some(&l) = layer.get(id) {
            if let Some(slot) = layers.get_mut(l) {
                slot.push(id.clone());
            }
        }
    }
    layers
}

fn render_flowchart_td(diag: &ParsedDiagram, width: usize) -> Vec<Line<'static>> {
    let layers = compute_layers(diag);
    let max_box_w = (width / 4).max(10).min(30);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " mermaid ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    let edge_map: HashMap<(String, String), &DiagEdge> = diag.edges.iter()
        .map(|e| ((e.from.clone(), e.to.clone()), e))
        .collect();

    for (li, layer) in layers.iter().enumerate() {
        let boxes: Vec<Vec<String>> = layer.iter().map(|id| {
            let node = diag.nodes.get(id);
            let (label, shape) = node.map(|n| (n.label.as_str(), n.shape)).unwrap_or((id.as_str(), NodeShape::Plain));
            render_box(label, shape, max_box_w)
        }).collect();

        if !boxes.is_empty() {
            let n_boxes = boxes.len();
            let box_h = boxes[0].len();
            for row in 0..box_h {
                let mut spans = Vec::new();
                for (bi, box_lines) in boxes.iter().enumerate() {
                    let line = box_lines.get(row).map(|s| s.as_str()).unwrap_or("");
                    spans.push(Span::styled(line.to_string(), box_border_style(layer, bi)));
                    if bi + 1 < n_boxes {
                        spans.push(Span::raw("   "));
                    }
                }
                lines.push(Line::from(spans));
            }
        }

        if li + 1 < layers.len() {
            let next_layer = &layers[li + 1];
            let connectors = compute_connectors(&layer, next_layer, &edge_map, diag);
            for conn_line in connectors {
                lines.push(Line::from(Span::styled(conn_line, Style::default().fg(Color::DarkGray))));
            }
        }
    }

    lines.push(Line::from(""));
    lines
}

fn box_border_style(_layer: &[String], _idx: usize) -> Style {
    Style::default().fg(Color::Cyan)
}

fn compute_connectors(
    from: &[String],
    to: &[String],
    edge_map: &HashMap<(String, String), &DiagEdge>,
    diag: &ParsedDiagram,
) -> Vec<String> {
    let mut connections: Vec<(usize, usize, Option<String>)> = Vec::new();
    for (fi, f_id) in from.iter().enumerate() {
        for (ti, t_id) in to.iter().enumerate() {
            if let Some(e) = edge_map.get(&(f_id.clone(), t_id.clone())) {
                connections.push((fi, ti, e.label.clone()));
            }
        }
    }

    if connections.is_empty() {
        return vec!["  │".to_string(), "  ▼".to_string()];
    }

    let n_from = from.len();
    let n_to = to.len();

    let mut lines = Vec::new();

    for &(fi, ti, ref label) in &connections {
        let label_str = label.as_ref().map(|l| format!(" {}", truncate_str(l, 20))).unwrap_or_default();
        lines.push(format!("  {}{}", "│", label_str));
    }
    lines.push("  ▼".to_string());

    lines
}

fn render_flowchart_lr(diag: &ParsedDiagram, width: usize) -> Vec<Line<'static>> {
    let layers = compute_layers(diag);
    let max_box_w = (width / 4).max(10).min(25);
    let max_box_w = max_box_w.min(width.saturating_sub(4).max(10));

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " mermaid ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    let max_nodes_in_layer = layers.iter().map(|l| l.len()).max().unwrap_or(1);
    let box_h = 3;

    for row in 0..max_nodes_in_layer {
        let mut row_boxes: Vec<Vec<String>> = Vec::new();
        for layer in &layers {
            if row < layer.len() {
                let id = &layer[row];
                let node = diag.nodes.get(id);
                let (label, shape) = node.map(|n| (n.label.as_str(), n.shape)).unwrap_or((id.as_str(), NodeShape::Plain));
                row_boxes.push(render_box(label, shape, max_box_w));
            } else {
                row_boxes.push(vec![" ".repeat(max_box_w + 4)]);
            }
        }

        for r in 0..box_h {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (bi, box_lines) in row_boxes.iter().enumerate() {
                let line = box_lines.get(r).map(|s| s.as_str()).unwrap_or("");
                spans.push(Span::styled(line.to_string(), Style::default().fg(Color::Cyan)));
                if bi + 1 < row_boxes.len() {
                    if r == 1 {
                        spans.push(Span::styled("───▶  ", Style::default().fg(Color::DarkGray)));
                    } else {
                        spans.push(Span::raw("      "));
                    }
                }
            }
            lines.push(Line::from(spans));
        }
    }

    lines.push(Line::from(""));
    lines
}

fn render_box(label: &str, shape: NodeShape, max_w: usize) -> Vec<String> {
    let label = if label.is_empty() { " " } else { label };
    let label_w = label.chars().count();
    let inner_w = label_w.min(max_w).max(3);
    let label_display: String = label.chars().take(inner_w).collect();
    let pad = if inner_w > label_display.chars().count() { 0 } else { 0 };
    let _ = pad;
    let inner_w = inner_w.max(label_display.chars().count());

    match shape {
        NodeShape::Box | NodeShape::Plain => {
            vec![
                format!("┌─{}─┐", "─".repeat(inner_w)),
                format!("│ {} │", center_text(&label_display, inner_w)),
                format!("└─{}─┘", "─".repeat(inner_w)),
            ]
        }
        NodeShape::Round => {
            vec![
                format!("╭─{}─╮", "─".repeat(inner_w)),
                format!("│ {} │", center_text(&label_display, inner_w)),
                format!("╰─{}─╯", "─".repeat(inner_w)),
            ]
        }
        NodeShape::Diamond => {
            vec![
                format!("  ◇{}◇  ", "─".repeat(inner_w)),
                format!("◇ {} ◇", center_text(&label_display, inner_w)),
                format!("  ◇{}◇  ", "─".repeat(inner_w)),
            ]
        }
        NodeShape::Circle => {
            vec![
                format!(" ╭{}╮ ", "─".repeat(inner_w + 2)),
                format!("( {} )", center_text(&label_display, inner_w)),
                format!(" ╰{}╯ ", "─".repeat(inner_w + 2)),
            ]
        }
    }
}

fn center_text(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.chars().take(width).collect();
    }
    let left = (width - len) / 2;
    let right = width - len - left;
    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
}

fn truncate_str(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn render_harvest_graph(source: &str, width: usize) -> Vec<Line<'static>> {
    let parsed: serde_json::Value = match serde_json::from_str(source) {
        Ok(v) => v,
        Err(_) => return render_code(source, "harvest-graph", width),
    };

    let symbols = parsed.get("symbols").and_then(|v| v.as_array());
    let relations = parsed.get("relations").and_then(|v| v.as_array());

    let Some(symbols) = symbols else {
        return render_code(source, "harvest-graph", width);
    };

    if symbols.is_empty() {
        return vec![
            Line::from(Span::styled(" harvest-graph ", Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC))),
            Line::from(""),
            Line::from(Span::styled("(empty graph)", Style::default().fg(Color::DarkGray))),
            Line::from(""),
        ];
    }

    let mut node_labels: HashMap<String, (String, String)> = HashMap::new();
    for s in symbols {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = s.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let file = s.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let short_file = file.rsplit('/').next().unwrap_or(file);
        let label = format!("{}\n{}", name, short_file);
        node_labels.insert(name.to_string(), (label, kind.to_string()));
    }

    let mut incoming: BTreeSet<String> = BTreeSet::new();
    let mut outgoing: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    if let Some(rels) = relations {
        for r in rels {
            let source_name = r.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let target_name = r.get("target").and_then(|v| v.as_str()).unwrap_or("");
            let rel = r.get("relation").and_then(|v| v.as_str()).unwrap_or("");
            if !source_name.is_empty() && !target_name.is_empty() {
                outgoing.entry(source_name.to_string()).or_default().push((target_name.to_string(), rel.to_string()));
                incoming.insert(target_name.to_string());
            }
        }
    }

    let roots: Vec<String> = node_labels.keys().filter(|k| !incoming.contains(*k)).cloned().collect();
    let max_box_w = (width / 3).max(10).min(25);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " harvest-graph ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    let mut visited = HashSet::new();
    for root in &roots {
        render_graph_node(root, &node_labels, &outgoing, 0, max_box_w, &mut visited, &mut lines);
    }
    for (name, _) in &node_labels {
        if !visited.contains(name) {
            render_graph_node(name, &node_labels, &outgoing, 0, max_box_w, &mut visited, &mut lines);
        }
    }

    lines.push(Line::from(""));
    lines
}

fn render_graph_node(
    name: &str,
    labels: &HashMap<String, (String, String)>,
    outgoing: &BTreeMap<String, Vec<(String, String)>>,
    depth: usize,
    max_w: usize,
    visited: &mut HashSet<String>,
    lines: &mut Vec<Line<'static>>,
) {
    if visited.contains(name) {
        return;
    }
    visited.insert(name.to_string());

    let indent = "  ".repeat(depth);
    let default_label = (name.to_string(), String::new());
    let (label, kind) = labels.get(name).unwrap_or(&default_label);
    let label_lines: Vec<&str> = label.lines().collect();

    let name_line = label_lines.first().unwrap_or(&name);
    let file_line = label_lines.get(1).unwrap_or(&"");

    let inner_w = name_line.chars().count().max(file_line.chars().count()).max(3).min(max_w);

    let kind_tag = if kind.is_empty() { "" } else { kind };

    lines.push(Line::from(vec![
        Span::styled(indent.clone(), Style::default()),
        Span::styled(format!("┌─{}─┐", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(indent.clone(), Style::default()),
        Span::styled("│ ", Style::default().fg(Color::Cyan)),
        Span::styled(center_text(name_line, inner_w), Style::default().fg(Color::Reset).add_modifier(Modifier::BOLD)),
        Span::styled(" │", Style::default().fg(Color::Cyan)),
    ]));
    if !file_line.is_empty() || !kind_tag.is_empty() {
        let meta = if !kind_tag.is_empty() && !file_line.is_empty() {
            format!("{}:{}", kind_tag, file_line)
        } else if !kind_tag.is_empty() {
            kind_tag.to_string()
        } else {
            file_line.to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(indent.clone(), Style::default()),
            Span::styled("│ ", Style::default().fg(Color::Cyan)),
            Span::styled(center_text(&meta, inner_w), Style::default().fg(Color::DarkGray)),
            Span::styled(" │", Style::default().fg(Color::Cyan)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(indent.clone(), Style::default()),
        Span::styled(format!("└─{}─┘", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
    ]));

    if let Some(targets) = outgoing.get(name) {
        for (target, rel) in targets {
            let arrow = if rel == "calls" { "──calls──▶" } else if rel == "uses" { "──uses──▶" } else if rel == "contains" { "──contains──▶" } else { "────▶" };
            lines.push(Line::from(vec![
                Span::styled(format!("{}  {}", indent, arrow), Style::default().fg(Color::DarkGray)),
                Span::styled(format!(" {}", rel), Style::default().fg(Color::DarkGray)),
            ]));
            render_graph_node(target, labels, outgoing, depth + 1, max_w, visited, lines);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_flowchart() {
        let src = "flowchart TD\n    A[Start] --> B[End]";
        let diag = parse_mermaid(src).unwrap();
        assert_eq!(diag.nodes.len(), 2);
        assert_eq!(diag.edges.len(), 1);
        assert_eq!(diag.direction, Direction::TD);
    }

    #[test]
    fn parses_circle_node() {
        let src = "graph TD\n    OS((Operating System)) --> Main[main]";
        let diag = parse_mermaid(src).unwrap();
        let os = diag.nodes.get("OS").unwrap();
        assert_eq!(os.shape, NodeShape::Circle);
        assert_eq!(os.label, "Operating System");
    }

    #[test]
    fn parses_dotted_edge() {
        let src = "graph TD\n    A -.-> B{Decision}";
        let diag = parse_mermaid(src).unwrap();
        assert_eq!(diag.edges.len(), 1);
        assert!(diag.edges[0].dotted);
        let b = diag.nodes.get("B").unwrap();
        assert_eq!(b.shape, NodeShape::Diamond);
    }

    #[test]
    fn parses_subgraph() {
        let src = "flowchart TD\n    A[Root] --> B[Child]\n    subgraph MyGroup\n        B\n    end";
        let diag = parse_mermaid(src).unwrap();
        assert_eq!(diag.subgraphs.len(), 1);
        assert_eq!(diag.subgraphs[0].0, "MyGroup");
    }

    #[test]
    fn renders_box() {
        let lines = render_box("Hello", NodeShape::Box, 20);
        assert!(lines[0].contains("┌"));
        assert!(lines[1].contains("Hello"));
        assert!(lines[2].contains("└"));
    }

    #[test]
    fn renders_circle() {
        let lines = render_box("Node", NodeShape::Circle, 20);
        assert!(lines[0].contains("╭"));
    }

    #[test]
    fn harvest_graph_renders() {
        let src = r#"{"symbols":[{"name":"main","kind":"function","file":"main.go"},{"name":"cmdDaemon","kind":"function","file":"main_daemon.go"}],"relations":[{"source":"main","target":"cmdDaemon","relation":"calls"}]}"#;
        let lines = render_harvest_graph(src, 80);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("main"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("calls"))));
    }
}
