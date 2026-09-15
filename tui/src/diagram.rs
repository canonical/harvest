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
    let first_line = source.lines().next().unwrap_or("").trim();
    let lower = first_line.to_lowercase();
    if lower.starts_with("flowchart") || lower.starts_with("graph ") || lower.starts_with("graph\t") {
        match parse_mermaid(source) {
            Some(p) => render_flowchart(&p, width),
            None => render_code(source, "mermaid", width),
        }
    } else if lower.starts_with("sequencediagram") {
        render_sequence(source, width)
    } else if lower.starts_with("classdiagram") {
        render_class_diagram(source, width)
    } else if lower.starts_with("statediagram") {
        render_state_diagram(source, width)
    } else if lower.starts_with("erdiagram") {
        render_er_diagram(source, width)
    } else if lower.starts_with("gantt") {
        render_gantt(source, width)
    } else if lower.starts_with("pie") {
        render_pie_chart(source, width)
    } else {
        render_code(source, "mermaid", width)
    }
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

struct SeqMessage {
    from: String,
    to: String,
    label: String,
    kind: SeqKind,
}

#[derive(Clone, Copy, PartialEq)]
enum SeqKind {
    Solid,
    Dashed,
    Failed,
}

struct SeqNote {
    over: Vec<String>,
    text: String,
    msg_index: usize,
}

struct SeqBlock {
    kind: String,
    label: String,
    start_msg: usize,
    end_msg: usize,
}

fn render_sequence(source: &str, width: usize) -> Vec<Line<'static>> {
    let mut participants: Vec<(String, String)> = Vec::new();
    let mut part_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut messages: Vec<SeqMessage> = Vec::new();
    let mut notes: Vec<SeqNote> = Vec::new();
    let mut blocks: Vec<SeqBlock> = Vec::new();
    let mut block_stack: Vec<(String, String, usize)> = Vec::new();
    let mut title = String::new();

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if line.starts_with("sequenceDiagram") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("title ") {
            title = rest.trim().to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("participant ") {
            let rest = rest.trim();
            if let Some(as_pos) = rest.find(" as ") {
                let id = rest[..as_pos].trim().to_string();
                let name = rest[as_pos + 4..].trim().to_string();
                part_map.insert(id.clone(), name.clone());
                participants.push((id, name));
            } else {
                let id = rest.to_string();
                part_map.insert(id.clone(), id.clone());
                participants.push((id.clone(), id));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("Note ") {
            let rest = rest.trim();
            let mut over_actors: Vec<String> = Vec::new();
            if let Some(pos) = rest.find(" over ") {
                let after = rest[pos + 6..].trim();
                let actors_part = if let Some(colon) = after.find(':') {
                    &after[..colon]
                } else {
                    after
                };
                for a in actors_part.split(',') {
                    let a = a.trim().to_string();
                    if !a.is_empty() {
                        over_actors.push(a);
                    }
                }
                let text = if let Some(colon) = after.find(':') {
                    after[colon + 1..].trim().to_string()
                } else {
                    String::new()
                };
                notes.push(SeqNote {
                    over: over_actors,
                    text,
                    msg_index: messages.len(),
                });
            }
            continue;
        }
        if line == "end" {
            if let Some((kind, label, start)) = block_stack.pop() {
                blocks.push(SeqBlock {
                    kind,
                    label,
                    start_msg: start,
                    end_msg: messages.len(),
                });
            }
            continue;
        }
        for b in ["loop", "alt", "opt", "else", "rect", "par", "critical"] {
            if line.starts_with(b) {
                let label = line[b.len()..].trim().to_string();
                block_stack.push((b.to_string(), label, messages.len()));
                break;
            }
        }
        if !block_stack.is_empty() && line.starts_with("else") {
            continue;
        }

        if let Some(colon) = line.find(':') {
            let left = line[..colon].trim();
            let text = line[colon + 1..].trim().to_string();

            let (from, to, kind) = if left.contains("->>") {
                let parts: Vec<&str> = left.splitn(2, "->>").collect();
                (parts[0].trim().to_string(), parts[1].trim().to_string(), SeqKind::Solid)
            } else if left.contains("-->>") {
                let parts: Vec<&str> = left.splitn(2, "-->>").collect();
                (parts[0].trim().to_string(), parts[1].trim().to_string(), SeqKind::Dashed)
            } else if left.contains("-x") || left.contains("--x") {
                let parts: Vec<&str> = left.splitn(2, "-x").collect();
                let to = if parts.len() > 1 { parts[1].trim() } else { "" };
                let to = if to.is_empty() {
                    if let Some(p) = left.splitn(2, "--x").nth(1) { p.trim() } else { "" }
                } else { to };
                (parts[0].trim().to_string(), to.to_string(), SeqKind::Failed)
            } else {
                continue;
            };

            for id in [&from, &to] {
                if !part_map.contains_key(id) {
                    part_map.insert(id.clone(), id.clone());
                    participants.push((id.clone(), id.clone()));
                }
            }

            messages.push(SeqMessage {
                from,
                to,
                label: text,
                kind,
            });
        }
    }

    if participants.is_empty() && messages.is_empty() {
        return render_code(source, "mermaid", width);
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " mermaid ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    if !title.is_empty() {
        lines.push(Line::from(Span::styled(
            title.clone(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
    }

    let n_parts = participants.len();
    if n_parts == 0 {
        return lines;
    }

    let col_width = (width / n_parts.max(1)).max(12).min(25);
    let col_width = col_width.min(width.saturating_sub(4).max(10));

    let col_centers: Vec<usize> = (0..n_parts)
        .map(|i| i * col_width + col_width / 2)
        .collect();

    let part_display: Vec<String> = participants.iter()
        .map(|(id, name)| part_map.get(id).cloned().unwrap_or_else(|| name.clone()))
        .collect();

    let header_w = col_width.saturating_sub(2).max(4);
    for i in 0..n_parts {
        let _ = i;
    }

    let mut header_line_spans: Vec<Span<'static>> = Vec::new();
    for (i, name) in part_display.iter().enumerate() {
        let display = if name.chars().count() > header_w {
            name.chars().take(header_w).collect::<String>()
        } else {
            name.clone()
        };
        let padded = center_text(&display, header_w);
        if i > 0 {
            header_line_spans.push(Span::raw(" "));
        }
        header_line_spans.push(Span::styled(format!("[{}]", padded), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    }
    lines.push(Line::from(header_line_spans));

    let mut lifeline_spans: Vec<Span<'static>> = Vec::new();
    for i in 0..n_parts {
        if i > 0 {
            lifeline_spans.push(Span::raw(" "));
        }
        lifeline_spans.push(Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray)));
    }
    let lifeline_template = Line::from(lifeline_spans);

    let lifeline_template_spans: Vec<Span<'static>> = (0..n_parts).flat_map(|i| {
        if i > 0 {
            vec![Span::raw(" "), Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray))]
        } else {
            vec![Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray))]
        }
    }).collect();
    let lifeline_line = Line::from(lifeline_template_spans);

    lines.push(lifeline_line.clone());

    for (mi, msg) in messages.iter().enumerate() {
        for note in &notes {
            if note.msg_index == mi {
                let note_spans = render_seq_note(note, &participants, &col_centers, header_w);
                for ns in note_spans {
                    lines.push(ns);
                }
            }
        }

        let from_idx = participants.iter().position(|(id, _)| id == &msg.from);
        let to_idx = participants.iter().position(|(id, _)| id == &msg.to);

        if from_idx.is_none() || to_idx.is_none() {
            continue;
        }

        let from_i = from_idx.unwrap();
        let to_i = to_idx.unwrap();

        let (arrow_char, arrow_color) = match msg.kind {
            SeqKind::Solid => ("─▶", Color::Reset),
            SeqKind::Dashed => ("··▶", Color::DarkGray),
            SeqKind::Failed => ("─✖", Color::Red),
        };

        let label = &msg.label;
        let label_trunc = truncate_str(label, col_width.saturating_sub(4).max(6));

        let mut msg_spans: Vec<Span<'static>> = Vec::new();
        let arrow_len: usize = 2;

        for i in 0..n_parts {
            if i > 0 {
                msg_spans.push(Span::raw(" "));
            }
            if i == from_i && i == to_i {
                msg_spans.push(Span::styled(format!("│ ╭──{}──╮ │", label_trunc.chars().take(header_w.saturating_sub(4).max(4)).collect::<String>()), Style::default().fg(Color::DarkGray)));
            } else if i == from_i || i == to_i {
                let min_c = col_centers[from_i].min(col_centers[to_i]);
                let max_c = col_centers[from_i].max(col_centers[to_i]);
                let _ = (min_c, max_c);
                msg_spans.push(Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray)));
            } else {
                msg_spans.push(Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray)));
            }
        }

        let mut arrow_row: Vec<Span<'static>> = Vec::new();
        let lo = from_i.min(to_i);
        let hi = from_i.max(to_i);

        for i in 0..n_parts {
            if i > 0 {
                arrow_row.push(Span::raw(" "));
            }
            if i == lo {
                arrow_row.push(Span::styled(format!("├{}┤", "─".repeat(header_w)), Style::default().fg(Color::DarkGray)));
            } else if i == hi {
                arrow_row.push(Span::styled(format!("├{}┤", "─".repeat(header_w)), Style::default().fg(Color::DarkGray)));
            } else if i > lo && i < hi {
                arrow_row.push(Span::styled(format!("├{}┤", "─".repeat(header_w)), Style::default().fg(Color::DarkGray)));
            } else {
                arrow_row.push(Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray)));
            }
        }

        let label_row: Vec<Span<'static>> = {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..n_parts {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                if i == lo {
                    let mid = col_centers[lo].saturating_sub(0);
                    let _ = mid;
                    spans.push(Span::styled(format!("  {}  ", center_text(&label_trunc, header_w)), Style::default().fg(arrow_color)));
                } else if i == hi {
                    spans.push(Span::styled(format!("  {}{}", arrow_char, " ".repeat(header_w.saturating_sub(arrow_len))), Style::default().fg(arrow_color)));
                } else if i > lo && i < hi {
                    spans.push(Span::styled(format!("  {}  ", "─".repeat(header_w)), Style::default().fg(Color::DarkGray)));
                } else {
                    spans.push(Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray)));
                }
            }
            spans
        };

        lines.push(Line::from(label_row));

        let from_left = from_i < to_i;
        let mut arrow_row2: Vec<Span<'static>> = Vec::new();
        for i in 0..n_parts {
            if i > 0 {
                arrow_row2.push(Span::raw(" "));
            }
            if i == lo {
                let a = if from_left { "├" } else { "┤" };
                arrow_row2.push(Span::styled(format!("{}{}{}", a, "─".repeat(header_w), "┤"), Style::default().fg(Color::DarkGray)));
            } else if i == hi {
                let a = if from_left { "├" } else { "┤" };
                arrow_row2.push(Span::styled(format!("{}{}{}", "├", "─".repeat(header_w), a), Style::default().fg(Color::DarkGray)));
            } else if i > lo && i < hi {
                arrow_row2.push(Span::styled(format!("├{}┤", "─".repeat(header_w)), Style::default().fg(Color::DarkGray)));
            } else {
                arrow_row2.push(Span::styled("│".repeat(header_w + 2), Style::default().fg(Color::DarkGray)));
            }
        }
        lines.push(Line::from(arrow_row2));
        lines.push(lifeline_line.clone());
    }

    for note in &notes {
        if note.msg_index >= messages.len() {
            let note_spans = render_seq_note(note, &participants, &col_centers, header_w);
            for ns in note_spans {
                lines.push(ns);
            }
        }
    }

    lines.push(Line::from(""));
    lines
}

fn render_seq_note(
    note: &SeqNote,
    participants: &[(String, String)],
    _col_centers: &[usize],
    header_w: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let text = &note.text;
    let w = header_w.min(40).max(8);
    out.push(Line::from(vec![
        Span::styled(format!("  ┌─{}─┐", "─".repeat(w)), Style::default().fg(Color::Yellow)),
    ]));
    for line in text.lines().take(3) {
        out.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(Color::Yellow)),
            Span::styled(center_text(line, w), Style::default().fg(Color::Reset)),
            Span::styled(" │", Style::default().fg(Color::Yellow)),
        ]));
    }
    out.push(Line::from(vec![
        Span::styled(format!("  └─{}─┘", "─".repeat(w)), Style::default().fg(Color::Yellow)),
    ]));
    out
}

struct ClassDef {
    name: String,
    attrs: Vec<String>,
    methods: Vec<String>,
}

struct ClassRel {
    from: String,
    to: String,
    rel_type: String,
    label: Option<String>,
}

fn render_class_diagram(source: &str, width: usize) -> Vec<Line<'static>> {
    let mut classes: BTreeMap<String, ClassDef> = BTreeMap::new();
    let mut class_order: Vec<String> = Vec::new();
    let mut rels: Vec<ClassRel> = Vec::new();
    let mut in_class = false;
    let mut current_name = String::new();
    let mut current_attrs: Vec<String> = Vec::new();
    let mut current_methods: Vec<String> = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with("classDiagram") {
            continue;
        }
        if line.starts_with("direction ") {
            continue;
        }
        if line.starts_with("class ") {
            let name = line["class ".len()..].trim().trim_end_matches('{').trim().to_string();
            if in_class {
                classes.insert(current_name.clone(), ClassDef {
                    name: current_name.clone(),
                    attrs: current_attrs.clone(),
                    methods: current_methods.clone(),
                });
                if !class_order.contains(&current_name) {
                    class_order.push(current_name.clone());
                }
            }
            current_name = name;
            current_attrs = Vec::new();
            current_methods = Vec::new();
            in_class = true;
            if line.ends_with('}') {
                in_class = false;
                classes.insert(current_name.clone(), ClassDef {
                    name: current_name.clone(),
                    attrs: current_attrs.clone(),
                    methods: current_methods.clone(),
                });
                if !class_order.contains(&current_name) {
                    class_order.push(current_name.clone());
                }
            }
            continue;
        }
        if line == "}" {
            if in_class {
                classes.insert(current_name.clone(), ClassDef {
                    name: current_name.clone(),
                    attrs: current_attrs.clone(),
                    methods: current_methods.clone(),
                });
                if !class_order.contains(&current_name) {
                    class_order.push(current_name.clone());
                }
                in_class = false;
            }
            continue;
        }
        if in_class {
            let member = line.trim_end_matches(';').trim();
            if member.contains('(') {
                current_methods.push(member.to_string());
            } else {
                current_attrs.push(member.to_string());
            }
            continue;
        }

        for (pat, rel_type) in [
            ("<|--", "inheritance"),
            ("*--", "composition"),
            ("o--", "aggregation"),
            ("..>", "dependency"),
            ("..|>", "realization"),
            ("<..", "reverse_dep"),
            ("-->", "association"),
        ] {
            if let Some(pos) = line.find(pat) {
                let from = line[..pos].trim().to_string();
                let rest = &line[pos + pat.len()..];
                let (to, label) = if let Some(colon) = rest.find(':') {
                    (rest[..colon].trim().to_string(), Some(rest[colon + 1..].trim().to_string()))
                } else {
                    (rest.trim().to_string(), None)
                };
                rels.push(ClassRel {
                    from,
                    to,
                    rel_type: rel_type.to_string(),
                    label,
                });
                break;
            }
        }
    }

    if in_class {
        classes.insert(current_name.clone(), ClassDef {
            name: current_name.clone(),
            attrs: current_attrs.clone(),
            methods: current_methods.clone(),
        });
        if !class_order.contains(&current_name) {
            class_order.push(current_name.clone());
        }
    }

    if classes.is_empty() && rels.is_empty() {
        return render_code(source, "mermaid", width);
    }

    for r in &rels {
        for name in [&r.from, &r.to] {
            if !classes.contains_key(name) {
                classes.insert(name.clone(), ClassDef {
                    name: name.clone(),
                    attrs: Vec::new(),
                    methods: Vec::new(),
                });
                class_order.push(name.clone());
            }
        }
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " mermaid ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    let max_box_w = (width / 3).max(12).min(30);

    for (name, cls) in &classes {
        let class_lines = render_class_box(name, cls, max_box_w);
        for l in class_lines {
            lines.push(l);
        }

        let rels_for: Vec<&ClassRel> = rels.iter().filter(|r| &r.from == name).collect();
        if !rels_for.is_empty() {
            for r in &rels_for {
                let rel_label = rel_short_label(&r.rel_type, r.label.as_deref());
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(format!("    {}────▶ {}", rel_label, r.to), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
        lines.push(Line::from(""));
    }

    lines
}

fn render_class_box(name: &str, cls: &ClassDef, max_w: usize) -> Vec<Line<'static>> {
    let name_w = name.chars().count();
    let attr_w = cls.attrs.iter().map(|a| a.chars().count()).max().unwrap_or(0);
    let method_w = cls.methods.iter().map(|m| m.chars().count()).max().unwrap_or(0);
    let inner_w = name_w.max(attr_w).max(method_w).min(max_w).max(4);

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(format!("  ┌─{}─┐", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::Cyan)),
        Span::styled(center_text(name, inner_w), Style::default().fg(Color::Reset).add_modifier(Modifier::BOLD)),
        Span::styled(" │", Style::default().fg(Color::Cyan)),
    ]));

    if !cls.attrs.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!("  ├─{}─┤", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
        ]));
        for attr in &cls.attrs {
            let display = truncate_str(attr, inner_w);
            lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(Color::Cyan)),
                Span::styled(center_text(&display, inner_w), Style::default().fg(Color::Reset)),
                Span::styled(" │", Style::default().fg(Color::Cyan)),
            ]));
        }
    }

    if !cls.methods.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!("  ├─{}─┤", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
        ]));
        for method in &cls.methods {
            let display = truncate_str(method, inner_w);
            lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(Color::Cyan)),
                Span::styled(center_text(&display, inner_w), Style::default().fg(Color::Reset)),
                Span::styled(" │", Style::default().fg(Color::Cyan)),
            ]));
        }
    }

    lines.push(Line::from(vec![
        Span::styled(format!("  └─{}─┘", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
    ]));

    lines
}

fn rel_short_label(rel_type: &str, label: Option<&str>) -> String {
    let marker = match rel_type {
        "inheritance" => "◀──ext",
        "composition" => "◀──cmp",
        "aggregation" => "◀──agg",
        "dependency" => "···dep",
        "realization" => "◀··rlz",
        "association" => "─────",
        "reverse_dep" => "···rev",
        _ => "─────",
    };
    match label {
        Some(l) => format!("{} ({})", marker, l),
        None => marker.to_string(),
    }
}

fn render_state_diagram(source: &str, width: usize) -> Vec<Line<'static>> {
    let mut nodes: BTreeMap<String, DiagNode> = BTreeMap::new();
    let mut edges: Vec<DiagEdge> = Vec::new();
    let mut node_order: Vec<String> = Vec::new();
    let mut subgraphs: Vec<(String, Vec<String>)> = Vec::new();
    let mut current_subgraph: Option<String> = None;

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with("stateDiagram") || line.starts_with("stateDiagram-v2") {
            continue;
        }
        if line.starts_with("note ") || line.starts_with("end note") {
            continue;
        }
        if line == "end" {
            current_subgraph = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("state ") {
            let name = rest.trim().trim_end_matches('{').trim().to_string();
            if !name.is_empty() && !nodes.contains_key(&name) {
                nodes.insert(name.clone(), DiagNode {
                    id: name.clone(),
                    label: name.clone(),
                    shape: NodeShape::Round,
                    subgraph: current_subgraph.clone(),
                });
                node_order.push(name.clone());
            }
            if line.ends_with('{') {
                current_subgraph = Some(name.clone());
                subgraphs.push((name, Vec::new()));
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("[*]") {
            let rest = rest.trim();
            let (target, label) = if let Some(pos) = rest.find("-->") {
                let after = rest[pos + 3..].trim();
                if let Some(colon) = after.find(':') {
                    (after[..colon].trim().to_string(), Some(after[colon + 1..].trim().to_string()))
                } else {
                    (after.to_string(), None)
                }
            } else {
                continue;
            };
            let start_id = "__start__".to_string();
            if !nodes.contains_key(&start_id) {
                nodes.insert(start_id.clone(), DiagNode {
                    id: start_id.clone(),
                    label: "Start".to_string(),
                    shape: NodeShape::Circle,
                    subgraph: None,
                });
                node_order.push(start_id.clone());
            }
            if !nodes.contains_key(&target) {
                nodes.insert(target.clone(), DiagNode {
                    id: target.clone(),
                    label: target.clone(),
                    shape: NodeShape::Round,
                    subgraph: current_subgraph.clone(),
                });
                node_order.push(target.clone());
            }
            edges.push(DiagEdge {
                from: start_id,
                to: target,
                label,
                dotted: false,
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("transition ") {
            let _ = rest;
            continue;
        }

        let (parsed_nodes, parsed_edges) = parse_line(line);
        for n in &parsed_nodes {
            let id = if n.id == "*" { "__end__".to_string() } else { n.id.clone() };
            if !nodes.contains_key(&id) {
                let mut node = n.clone();
                node.id = id.clone();
                if n.id == "*" {
                    node.label = "End".to_string();
                    node.shape = NodeShape::Circle;
                }
                node.subgraph = current_subgraph.clone();
                nodes.insert(id.clone(), node);
                node_order.push(id.clone());
                if let Some(sg) = &current_subgraph {
                    if let Some(entry) = subgraphs.iter_mut().find(|(n, _)| n == sg) {
                        entry.1.push(id.clone());
                    }
                }
            }
        }
        for e in &parsed_edges {
            let from = if e.from == "*" { "__end__".to_string() } else { e.from.clone() };
            let to = if e.to == "*" { "__end__".to_string() } else { e.to.clone() };
            for id in [&from, &to] {
                if !nodes.contains_key(id) {
                    let label = if id == "__end__" { "End".to_string() } else { id.clone() };
                    let shape = if id == "__end__" || id == "__start__" { NodeShape::Circle } else { NodeShape::Round };
                    nodes.insert(id.clone(), DiagNode {
                        id: id.clone(),
                        label,
                        shape,
                        subgraph: current_subgraph.clone(),
                    });
                    node_order.push(id.clone());
                }
            }
            edges.push(DiagEdge {
                from,
                to,
                label: e.label.clone(),
                dotted: e.dotted,
            });
        }
    }

    let diag = ParsedDiagram {
        direction: Direction::TD,
        nodes,
        edges,
        node_order,
        subgraphs,
    };

    render_flowchart_td(&diag, width)
}

struct ErEntity {
    name: String,
    attrs: Vec<(String, String)>,
}

struct ErRel {
    from: String,
    to: String,
    cardinality: String,
    label: String,
}

fn render_er_diagram(source: &str, width: usize) -> Vec<Line<'static>> {
    let mut entities: BTreeMap<String, ErEntity> = BTreeMap::new();
    let mut entity_order: Vec<String> = Vec::new();
    let mut rels: Vec<ErRel> = Vec::new();
    let mut in_entity = false;
    let mut current_name = String::new();
    let mut current_attrs: Vec<(String, String)> = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with("erDiagram") {
            continue;
        }
        if line == "}" {
            if in_entity {
                entities.insert(current_name.clone(), ErEntity {
                    name: current_name.clone(),
                    attrs: current_attrs.clone(),
                });
                if !entity_order.contains(&current_name) {
                    entity_order.push(current_name.clone());
                }
                in_entity = false;
            }
            continue;
        }

        if !in_entity && !line.contains("--") && !line.contains("..") {
            let name = line.trim_end_matches('{').trim().to_string();
            if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_alphabetic()) {
                current_name = name;
                current_attrs = Vec::new();
                in_entity = true;
                if line.ends_with('}') {
                    in_entity = false;
                    entities.insert(current_name.clone(), ErEntity {
                        name: current_name.clone(),
                        attrs: Vec::new(),
                    });
                    if !entity_order.contains(&current_name) {
                        entity_order.push(current_name.clone());
                    }
                }
                continue;
            }
        }

        if in_entity {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let attr_type = parts[0].to_string();
                let attr_name = parts[1].trim_end_matches(',').to_string();
                current_attrs.push((attr_type, attr_name));
            }
            continue;
        }

        let card_patterns = [
            "||--o{", "||--|{", "||--||", "}o--o{", "}o--||", "}o--|{",
            "|o--o{", "|o--|{", "|o--||",
        ];
        for card in card_patterns {
            if let Some(pos) = line.find(card) {
                let from = line[..pos].trim().to_string();
                let rest = &line[pos + card.len()..];
                let (to, label) = if let Some(colon) = rest.find(':') {
                    (rest[..colon].trim().to_string(), rest[colon + 1..].trim().to_string())
                } else {
                    (rest.trim().to_string(), String::new())
                };
                rels.push(ErRel {
                    from,
                    to,
                    cardinality: card.to_string(),
                    label,
                });
                break;
            }
        }
    }

    if in_entity {
        entities.insert(current_name.clone(), ErEntity {
            name: current_name.clone(),
            attrs: current_attrs.clone(),
        });
        if !entity_order.contains(&current_name) {
            entity_order.push(current_name.clone());
        }
    }

    if entities.is_empty() && rels.is_empty() {
        return render_code(source, "mermaid", width);
    }

    for r in &rels {
        for name in [&r.from, &r.to] {
            if !entities.contains_key(name) {
                entities.insert(name.clone(), ErEntity {
                    name: name.clone(),
                    attrs: Vec::new(),
                });
                entity_order.push(name.clone());
            }
        }
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " mermaid ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    let max_box_w = (width / 3).max(12).min(30);

    for (name, entity) in &entities {
        let entity_lines = render_er_box(name, entity, max_box_w);
        for l in entity_lines {
            lines.push(l);
        }

        let rels_for: Vec<&ErRel> = rels.iter().filter(|r| &r.from == name).collect();
        if !rels_for.is_empty() {
            for r in &rels_for {
                let card_label = card_short(&r.cardinality);
                let label = if r.label.is_empty() {
                    card_label.to_string()
                } else {
                    format!("{} ({})", card_label, r.label)
                };
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(format!("    {}────▶ {}", label, r.to), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
        lines.push(Line::from(""));
    }

    lines
}

fn render_er_box(name: &str, entity: &ErEntity, max_w: usize) -> Vec<Line<'static>> {
    let name_w = name.chars().count();
    let attr_w = entity.attrs.iter().map(|(t, n)| format!("{} {}", t, n).chars().count()).max().unwrap_or(0);
    let inner_w = name_w.max(attr_w).min(max_w).max(4);

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(format!("  ┌─{}─┐", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::Cyan)),
        Span::styled(center_text(name, inner_w), Style::default().fg(Color::Reset).add_modifier(Modifier::BOLD)),
        Span::styled(" │", Style::default().fg(Color::Cyan)),
    ]));

    if !entity.attrs.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!("  ├─{}─┤", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
        ]));
        for (attr_type, attr_name) in &entity.attrs {
            let display = format!("{} {}", attr_type, attr_name);
            let truncated = truncate_str(&display, inner_w);
            lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(Color::Cyan)),
                Span::styled(center_text(&truncated, inner_w), Style::default().fg(Color::Reset)),
                Span::styled(" │", Style::default().fg(Color::Cyan)),
            ]));
        }
    }

    lines.push(Line::from(vec![
        Span::styled(format!("  └─{}─┘", "─".repeat(inner_w)), Style::default().fg(Color::Cyan)),
    ]));

    lines
}

fn card_short(card: &str) -> &'static str {
    match card {
        "||--||" => "1:1",
        "||--o{" => "1:0+",
        "||--|{" => "1:1+",
        "}o--o{" => "0+:0+",
        "}o--||" => "0+:1",
        "}o--|{" => "0+:1+",
        "|o--o{" => "0-1:0+",
        "|o--|{" => "0-1:1+",
        "|o--||" => "0-1:1",
        _ => "—",
    }
}

struct GanttTask {
    name: String,
    start_day: i64,
    duration: i64,
    section: String,
}

fn render_gantt(source: &str, width: usize) -> Vec<Line<'static>> {
    let mut tasks: Vec<GanttTask> = Vec::new();
    let mut title = String::new();
    let mut current_section = String::new();
    let mut day_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

    let date_to_day = |s: &str| -> i64 {
        let s = s.trim();
        if let Ok(d) = s.parse::<i64>() {
            return d;
        }
        if s.len() >= 6 {
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(day) = parts[0].parse::<i64>() {
                    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                    let month_idx = months.iter().position(|m| *m == parts[1]).map(|i| i as i64).unwrap_or(0);
                    let year: i64 = parts[2].parse().unwrap_or(2000);
                    return (year - 2000) * 365 + month_idx * 30 + day;
                }
            }
        }
        0
    };

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with("gantt") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("title ") {
            title = rest.trim().to_string();
            continue;
        }
        if line.starts_with("dateFormat ") || line.starts_with("axisFormat ") || line.starts_with("excludes ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("section ") {
            current_section = rest.trim().to_string();
            continue;
        }

        if let Some(colon) = line.find(':') {
            let name = line[..colon].trim().to_string();
            let rest = line[colon + 1..].trim();

            let parts: Vec<&str> = rest.split(',').collect();
            if parts.is_empty() {
                continue;
            }

            let (start_day, duration) = if parts.len() >= 3 {
                let start = date_to_day(parts[0].trim());
                let dur = parts[1].trim().trim_end_matches('d').parse::<i64>().unwrap_or(1);
                (start, dur)
            } else if parts.len() == 2 {
                let first = parts[0].trim();
                if first.starts_with("after ") {
                    let dep = first["after ".len()..].trim().to_string();
                    let start = day_map.get(&dep).copied().unwrap_or(0);
                    let dur = parts[1].trim().trim_end_matches('d').parse::<i64>().unwrap_or(1);
                    (start, dur)
                } else {
                    let start = date_to_day(first);
                    let dur = parts[1].trim().trim_end_matches('d').parse::<i64>().unwrap_or(1);
                    (start, dur)
                }
            } else if parts.len() == 1 {
                let first = parts[0].trim();
                if first.starts_with("after ") {
                    let dep = first["after ".len()..].trim().to_string();
                    let start = day_map.get(&dep).copied().unwrap_or(0);
                    (start, 1)
                } else {
                    let dur = first.trim_end_matches('d').parse::<i64>().unwrap_or(1);
                    (0, dur)
                }
            } else {
                (0, 1)
            };

            let task = GanttTask {
                name: name.clone(),
                start_day,
                duration,
                section: current_section.clone(),
            };
            day_map.insert(name, start_day + duration);
            tasks.push(task);
        }
    }

    if tasks.is_empty() {
        return render_code(source, "mermaid", width);
    }

    let min_start = tasks.iter().map(|t| t.start_day).min().unwrap_or(0);
    let max_end = tasks.iter().map(|t| t.start_day + t.duration).max().unwrap_or(1);
    let total_span = (max_end - min_start).max(1);

    let max_bar_w = (width as i64).saturating_sub(30).max(20) as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " mermaid ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    if !title.is_empty() {
        lines.push(Line::from(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));
    }

    let mut sections: Vec<String> = Vec::new();
    let mut current_sec = String::new();
    for t in &tasks {
        if t.section != current_sec {
            current_sec = t.section.clone();
            sections.push(current_sec.clone());
        }
    }

    let mut sec_idx = 0;
    for t in &tasks {
        if t.section != sections.get(sec_idx).map(|s| s.as_str()).unwrap_or("") {
            sec_idx += 1;
            if sec_idx < sections.len() {
                lines.push(Line::from(Span::styled(
                    sections[sec_idx].clone(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )));
            }
        } else if sec_idx == 0 && !sections.is_empty() {
            lines.push(Line::from(Span::styled(
                sections[0].clone(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
            sec_idx = 1;
        }

        let offset = (t.start_day - min_start).max(0) as usize;
        let bar_len = ((t.duration as f64 / total_span as f64) * max_bar_w as f64).ceil() as usize;
        let bar_len = bar_len.max(1);

        let name_display = truncate_str(&t.name, 20);
        let prefix = format!("  {:<20} ", name_display);
        let padding = " ".repeat(offset.min(max_bar_w));
        let bar = "█".repeat(bar_len.min(max_bar_w.saturating_sub(offset)));

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::Reset)),
            Span::styled(padding, Style::default()),
            Span::styled(bar, Style::default().fg(Color::Blue)),
            Span::styled(format!("  {}d", t.duration), Style::default().fg(Color::DarkGray)),
        ]));
    }

    lines.push(Line::from(""));
    lines
}

fn render_pie_chart(source: &str, width: usize) -> Vec<Line<'static>> {
    let mut title = String::new();
    let mut entries: Vec<(String, f64)> = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("%%") || line == "pie" {
            continue;
        }
        if let Some(rest) = line.strip_prefix("title ") {
            title = rest.trim().to_string();
            continue;
        }
        if let Some(colon) = line.find(':') {
            let label_part = line[..colon].trim();
            let value_part = line[colon + 1..].trim();
            let label = label_part.trim_matches('"').to_string();
            let value: f64 = value_part.parse().unwrap_or(0.0);
            if value > 0.0 {
                entries.push((label, value));
            }
        }
    }

    if entries.is_empty() {
        return render_code(source, "mermaid", width);
    }

    let total: f64 = entries.iter().map(|(_, v)| v).sum();
    if total <= 0.0 {
        return render_code(source, "mermaid", width);
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        " mermaid ",
        Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    if !title.is_empty() {
        lines.push(Line::from(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));
    }

    let max_bar_w = (width.saturating_sub(30)).max(20);
    let colors = [Color::Blue, Color::Green, Color::Yellow, Color::Magenta, Color::Cyan, Color::Red];
    let label_w = 15;

    for (i, (label, value)) in entries.iter().enumerate() {
        let pct = (value / total * 100.0).round() as usize;
        let bar_len = (pct * max_bar_w / 100).max(1);
        let color = colors[i % colors.len()];
        let label_display = truncate_str(label, label_w);
        let padded_label = format!("{:<width$}", label_display, width = label_w);

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", padded_label), Style::default().fg(Color::Reset)),
            Span::styled("█".repeat(bar_len), Style::default().fg(color)),
            Span::styled(format!("  {}%", pct), Style::default().fg(Color::DarkGray)),
        ]));
    }

    lines.push(Line::from(""));
    lines
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

    #[test]
    fn renders_sequence_diagram() {
        let src = "sequenceDiagram\n    participant A as User\n    participant B as API\n    A->>B: Login\n    B-->>A: Token";
        let lines = render_mermaid(src, 120);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("User"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("API"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Login"))));
    }

    #[test]
    fn renders_class_diagram() {
        let src = "classDiagram\n    class Animal {\n      +name: String\n      +makeSound()\n    }\n    class Dog {\n      +fetch()\n    }\n    Animal <|-- Dog";
        let lines = render_mermaid(src, 100);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Animal"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Dog"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("ext"))));
    }

    #[test]
    fn renders_state_diagram() {
        let src = "stateDiagram-v2\n    [*] --> Idle\n    Idle --> Processing: Start\n    Processing --> Done\n    Done --> [*]";
        let lines = render_mermaid(src, 100);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Idle"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Processing"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Start"))));
    }

    #[test]
    fn renders_er_diagram() {
        let src = "erDiagram\n    USER {\n      int id\n      string email\n    }\n    POST {\n      int id\n      string title\n    }\n    USER ||--o{ POST : creates";
        let lines = render_mermaid(src, 100);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("USER"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("POST"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("1:0+"))));
    }

    #[test]
    fn renders_gantt() {
        let src = "gantt\n    title Project Timeline\n    section Planning\n    Research :0, 30d\n    Design :30, 20d\n    section Development\n    Code :50, 60d";
        let lines = render_mermaid(src, 100);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Research"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Code"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("█"))));
    }

    #[test]
    fn renders_pie_chart() {
        let src = "pie title Pet Ownership\n    \"Dogs\" : 52\n    \"Cats\" : 34\n    \"Fish\" : 14";
        let lines = render_mermaid(src, 100);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("Dogs"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("52%"))));
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("█"))));
    }

    #[test]
    fn unknown_mermaid_falls_back_to_code() {
        let src = "journey\n    title My working day\n    Go to work: 5";
        let lines = render_mermaid(src, 80);
        assert!(lines.iter().any(|l| l.spans.iter().any(|s| s.content.contains("journey"))));
    }
}
