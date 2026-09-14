use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn render_markdown(input: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_buf: Vec<String> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();

    let flush_para = |para: &mut Vec<String>, out: &mut Vec<Line<'static>>, w: usize| {
        if para.is_empty() {
            return;
        }
        let joined = para.join(" ");
        *para = Vec::new();
        for chunk in wrap_text(&joined, w) {
            out.push(Line::from(Span::styled(
                chunk,
                Style::default().fg(Color::Reset),
            )));
        }
        out.push(Line::from(""));
    };

    for raw in input.lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            if in_code {
                for cl in &code_buf {
                    lines.push(Line::from(Span::styled(
                        cl.clone(),
                        Style::default().fg(Color::Green),
                    )));
                }
                lines.push(Line::from(""));
                code_buf.clear();
                code_lang.clear();
                in_code = false;
            } else {
                flush_para(&mut paragraph, &mut lines, width);
                in_code = true;
                code_lang = line
                    .trim_start()
                    .trim_start_matches('`')
                    .trim()
                    .to_string();
                if !code_lang.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!(" {} ", code_lang),
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
            }
            continue;
        }
        if in_code {
            code_buf.push(line.to_string());
            continue;
        }
        if line.is_empty() {
            flush_para(&mut paragraph, &mut lines, width);
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            flush_para(&mut paragraph, &mut lines, width);
            for w in wrap_text(rest.trim(), width.saturating_sub(2)) {
                lines.push(Line::from(Span::styled(
                    w,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            lines.push(Line::from(""));
        } else if let Some(rest) = line.strip_prefix("## ") {
            flush_para(&mut paragraph, &mut lines, width);
            for w in wrap_text(rest.trim(), width.saturating_sub(2)) {
                lines.push(Line::from(Span::styled(
                    w,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            lines.push(Line::from(""));
        } else if let Some(rest) = line.strip_prefix("### ") {
            flush_para(&mut paragraph, &mut lines, width);
            for w in wrap_text(rest.trim(), width.saturating_sub(2)) {
                lines.push(Line::from(Span::styled(
                    w,
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            lines.push(Line::from(""));
        } else if line.starts_with("- ") || line.starts_with("* ") {
            flush_para(&mut paragraph, &mut lines, width);
            let item = line[2..].trim();
            for (i, w) in wrap_text(item, width.saturating_sub(2)).into_iter().enumerate() {
                let prefix = if i == 0 { "  • " } else { "    " };
                lines.push(Line::from(Span::styled(
                    format!("{prefix}{w}"),
                    Style::default().fg(Color::Reset),
                )));
            }
        } else if let Some(idx) = line.find(". ") {
            if line[..idx].chars().all(|c| c.is_ascii_digit()) {
                flush_para(&mut paragraph, &mut lines, width);
                let item = line[idx + 2..].trim();
                for w in wrap_text(item, width.saturating_sub(3)) {
                    lines.push(Line::from(Span::styled(
                        format!("  {w}"),
                        Style::default().fg(Color::Reset),
                    )));
                }
                continue;
            }
            paragraph.push(line.to_string());
        } else {
            paragraph.push(line.to_string());
        }
    }

    if in_code {
        for cl in &code_buf {
            lines.push(Line::from(Span::styled(
                cl.clone(),
                Style::default().fg(Color::Green),
            )));
        }
    } else {
        flush_para(&mut paragraph, &mut lines, width);
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
