use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use markdown2pdf::config::ConfigSource;
use markdown2pdf::fonts::{FontConfig, FontSource};
use std::collections::HashMap;
use std::path::PathBuf;

fn lowercase_roman(mut n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    const VALUES: [(usize, &str); 13] = [
        (1000, "m"), (900, "cm"), (500, "d"), (400, "cd"),
        (100, "c"), (90, "xc"), (50, "l"), (40, "xl"),
        (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i"),
    ];
    let mut out = String::new();
    for (value, symbol) in VALUES {
        while n >= value {
            out.push_str(symbol);
            n -= value;
        }
    }
    out
}

fn strip_inline_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' => {}
            '[' => {
                while let Some(&next) = chars.peek() {
                    if next == ']' {
                        chars.next();
                        break;
                    }
                    out.push(next);
                    chars.next();
                }
                if chars.peek() == Some(&'(') {
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next == ')' {
                            break;
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

fn is_fence_delimiter(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn parse_toc_headings(markdown: &str, max_level: u8) -> Vec<(u8, String)> {
    let mut headings = Vec::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if is_fence_delimiter(trimmed) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if hashes == 0 || hashes as u8 > max_level {
            continue;
        }
        let rest = trimmed[hashes..].trim_start();
        if rest.is_empty() || !trimmed[hashes..].starts_with(' ') {
            continue;
        }
        headings.push((hashes as u8, strip_inline_markdown(rest)));
    }
    headings
}

fn decode_pdf_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

struct OutlineEntry {
    level: u8,
    text: String,
    page_number: u32,
}

fn extract_outline_entries(doc: &Document) -> Vec<OutlineEntry> {
    let pages = doc.get_pages();
    let page_number_of = |id: ObjectId| pages.iter().find(|(_, v)| **v == id).map(|(k, _)| *k);

    let mut entries = Vec::new();
    let Ok(catalog) = doc.catalog() else { return entries };
    let Ok(outlines_id) = catalog.get(b"Outlines").and_then(Object::as_reference) else { return entries };
    let Ok(outlines) = doc.get_dictionary(outlines_id) else { return entries };

    let mut stack: Vec<ObjectId> = Vec::new();
    if let Ok(first) = outlines.get(b"First").and_then(Object::as_reference) {
        stack.push(first);
    }
    let mut visited = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Ok(item) = doc.get_dictionary(id) else { continue };
        if let Ok(next) = item.get(b"Next").and_then(Object::as_reference) {
            stack.push(next);
        }

        let title_bytes = item.get(b"Title").and_then(Object::as_str).unwrap_or_default();
        let title = decode_pdf_text(title_bytes);
        let indent = title.chars().take_while(|c| *c == ' ').count() / 2;
        let level = (indent + 1) as u8;
        let text = title.trim_start().to_string();

        let dest_page = item.get(b"Dest").ok().and_then(|d| d.as_array().ok()).and_then(|arr| arr.first());
        let page_id = dest_page.and_then(|o| o.as_reference().ok());
        if let Some(page_id) = page_id {
            if let Some(page_number) = page_number_of(page_id) {
                entries.push(OutlineEntry { level, text, page_number });
            }
        }
    }
    entries
}

fn heading_pages(markdown: &str, doc: &Document, max_level: u8) -> Vec<(u8, String, u32)> {
    let headings = parse_toc_headings(markdown, max_level);
    let outline = extract_outline_entries(doc);
    let mut lookup: HashMap<(u8, String), std::collections::VecDeque<u32>> = HashMap::new();
    for entry in outline {
        lookup.entry((entry.level, entry.text)).or_default().push_back(entry.page_number);
    }
    headings
        .into_iter()
        .filter_map(|(level, text)| {
            let page = lookup.get_mut(&(level, text.clone())).and_then(|q| q.pop_front())?;
            Some((level, text, page))
        })
        .collect()
}

fn font_asset_path(filename: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/fonts").join(filename)
}

fn ubuntu_font_config() -> FontConfig {
    FontConfig::new()
        .with_default_font_source(FontSource::File(font_asset_path("Ubuntu.ttf")))
}

fn render_markdown(markdown: &str, config_toml: &str) -> Result<Vec<u8>, String> {
    let font_config = ubuntu_font_config();
    markdown2pdf::parse_into_bytes(markdown.to_string(), ConfigSource::Embedded(config_toml), Some(&font_config))
        .map_err(|e| e.to_string())
}

fn merge_documents(mut docs: Vec<Document>) -> Document {
    let mut target = Document::with_version("1.5");
    let mut page_ids: Vec<ObjectId> = Vec::new();
    let mut next_id: u32 = 1;

    for doc in &mut docs {
        doc.renumber_objects_with(next_id);
        next_id = doc.max_id + 1;
        for (_, page_id) in doc.get_pages() {
            page_ids.push(page_id);
        }
        target.objects.append(&mut doc.objects);
    }
    target.max_id = next_id - 1;

    let pages_id = target.new_object_id();
    for page_id in &page_ids {
        if let Ok(page_dict) = target.get_dictionary_mut(*page_id) {
            page_dict.set("Parent", Object::Reference(pages_id));
        }
    }

    let mut pages_dict = Dictionary::new();
    pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
    pages_dict.set("Count", Object::Integer(page_ids.len() as i64));
    pages_dict.set("Kids", Object::Array(page_ids.iter().map(|id| Object::Reference(*id)).collect()));
    target.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = target.add_object(Object::Dictionary(catalog));

    target.trailer.set("Root", Object::Reference(catalog_id));
    target.max_id = target.max_id.max(catalog_id.0);
    target
}

const PAGE_NUMBER_FONT_KEY: &[u8] = b"HarvestPageNum";

fn helvetica_text_width(text: &str, size_pt: f32) -> f32 {
    let width_1000 = |c: char| -> f32 {
        match c {
            '0'..='9' => 556.0,
            'i' | 'l' => 222.0,
            'v' | 'x' => 500.0,
            'm' => 833.0,
            'c' | 'd' => 667.0,
            _ => 556.0,
        }
    };
    text.chars().map(width_1000).sum::<f32>() * size_pt / 1000.0
}

fn page_resources_id(doc: &mut Document, page_id: ObjectId) -> Result<ObjectId, String> {
    let page_dict = doc.get_dictionary(page_id).map_err(|e| e.to_string())?.clone();
    match page_dict.get(b"Resources") {
        Ok(Object::Reference(id)) => Ok(*id),
        _ => {
            let new_id = doc.new_object_id();
            let resources = page_dict.get(b"Resources").and_then(Object::as_dict).cloned().unwrap_or_default();
            doc.objects.insert(new_id, Object::Dictionary(resources));
            let page_dict_mut = doc.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
            page_dict_mut.set("Resources", Object::Reference(new_id));
            Ok(new_id)
        }
    }
}

fn add_resource_entry(
    doc: &mut Document,
    resources_id: ObjectId,
    category: &[u8],
    key: &[u8],
    value_id: ObjectId,
) -> Result<(), String> {
    let category_entry = doc.get_dictionary(resources_id).map_err(|e| e.to_string())?.get(category).cloned().ok();
    match category_entry {
        Some(Object::Reference(sub_id)) => {
            let sub = doc.get_dictionary_mut(sub_id).map_err(|e| e.to_string())?;
            sub.set(key, Object::Reference(value_id));
        }
        Some(Object::Dictionary(mut sub)) => {
            sub.set(key, Object::Reference(value_id));
            let resources = doc.get_dictionary_mut(resources_id).map_err(|e| e.to_string())?;
            resources.set(category, Object::Dictionary(sub));
        }
        _ => {
            let mut sub = Dictionary::new();
            sub.set(key, Object::Reference(value_id));
            let resources = doc.get_dictionary_mut(resources_id).map_err(|e| e.to_string())?;
            resources.set(category, Object::Dictionary(sub));
        }
    }
    Ok(())
}

fn add_isolated_page_contents(doc: &mut Document, page_id: ObjectId, content: &str) -> Result<(), String> {
    let mut isolated = String::with_capacity(content.len() + 1);
    isolated.push('\n');
    isolated.push_str(content);
    doc.add_page_contents(page_id, isolated.into_bytes()).map_err(|e| e.to_string())
}

fn ensure_page_font_resource(doc: &mut Document, page_id: ObjectId) -> Result<(), String> {
    let font_dict = {
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Font".to_vec()));
        d.set("Subtype", Object::Name(b"Type1".to_vec()));
        d.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
        d
    };
    let font_id = doc.add_object(Object::Dictionary(font_dict));
    let resources_id = page_resources_id(doc, page_id)?;
    add_resource_entry(doc, resources_id, b"Font", PAGE_NUMBER_FONT_KEY, font_id)
}

fn page_media_box(doc: &Document, page_id: ObjectId) -> (f32, f32) {
    doc.get_dictionary(page_id)
        .ok()
        .and_then(|d| d.get(b"MediaBox").ok())
        .and_then(|o| o.as_array().ok())
        .and_then(|arr| {
            let nums: Vec<f32> = arr.iter().filter_map(|o| o.as_float().ok()).collect();
            if nums.len() == 4 { Some((nums[2], nums[3])) } else { None }
        })
        .unwrap_or((595.0, 842.0))
}

fn stamp_page_label(doc: &mut Document, page_id: ObjectId, label: &str) -> Result<(), String> {
    ensure_page_font_resource(doc, page_id)?;

    let media_box = page_media_box(doc, page_id);

    let size_pt = 9.0;
    let text_width = helvetica_text_width(label, size_pt);
    let x = ((media_box.0 - text_width) / 2.0).max(0.0);
    let y = 28.0;

    let mut escaped = String::with_capacity(label.len());
    for c in label.chars() {
        if c == '(' || c == ')' || c == '\\' {
            escaped.push('\\');
        }
        escaped.push(c);
    }

    let content = format!(
        "q\nBT\n/{} {size_pt} Tf\n1 0 0 1 {x} {y} Tm\n({escaped}) Tj\nET\nQ\n",
        std::str::from_utf8(PAGE_NUMBER_FONT_KEY).unwrap(),
    );
    add_isolated_page_contents(doc, page_id, &content)
}

const UBUNTU_LOGO_RGB: &[u8] = include_bytes!("../../assets/images/ubuntu-logo.rgb");
const UBUNTU_LOGO_WIDTH_PX: i64 = 160;
const UBUNTU_LOGO_HEIGHT_PX: i64 = 275;
const UBUNTU_LOGO_KEY: &[u8] = b"HarvestUbuntuLogo";

fn stamp_logo(doc: &mut Document, page_id: ObjectId) -> Result<(), String> {
    debug_assert_eq!(
        (UBUNTU_LOGO_WIDTH_PX * UBUNTU_LOGO_HEIGHT_PX * 3) as usize,
        UBUNTU_LOGO_RGB.len(),
    );

    let mut image_dict = Dictionary::new();
    image_dict.set("Type", Object::Name(b"XObject".to_vec()));
    image_dict.set("Subtype", Object::Name(b"Image".to_vec()));
    image_dict.set("Width", Object::Integer(UBUNTU_LOGO_WIDTH_PX));
    image_dict.set("Height", Object::Integer(UBUNTU_LOGO_HEIGHT_PX));
    image_dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    image_dict.set("BitsPerComponent", Object::Integer(8));
    let mut image_stream = Stream::new(image_dict, UBUNTU_LOGO_RGB.to_vec());
    let _ = image_stream.compress();
    let image_id = doc.add_object(Object::Stream(image_stream));

    let resources_id = page_resources_id(doc, page_id)?;
    add_resource_entry(doc, resources_id, b"XObject", UBUNTU_LOGO_KEY, image_id)?;

    let media_box = page_media_box(doc, page_id);
    let margin_left = 16.0;
    let width = 36.0;
    let height = width * (UBUNTU_LOGO_HEIGHT_PX as f32 / UBUNTU_LOGO_WIDTH_PX as f32);
    let x = margin_left;
    let y = media_box.1 - height;

    let content = format!(
        "q\n{width} 0 0 {height} {x} {y} cm\n/{} Do\nQ\n",
        std::str::from_utf8(UBUNTU_LOGO_KEY).unwrap(),
    );
    add_isolated_page_contents(doc, page_id, &content)
}

pub struct TitlePageInfo {
    pub company: String,
    pub product: String,
    pub deployment_name: String,
    pub generated_date: String,
}

fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

const CANONICAL_THEME_CONFIG: &str = r##"
theme = "modern"

[paragraph]
text_align = "justify"

[headings.h1]
text_color = "#1B1F23"
[headings.h2]
text_color = "#1B1F23"
[headings.h3]
text_color = "#1B1F23"

[link]
text_color = "#E95420"

[horizontal_rule]
color = "#E95420"
thickness_pt = 1.5

[image]
max_width_pct = 70.0

[table]
row_gap_pt = 0.0
alternating_row_background = "#F7F3F0"
cell_padding = { top = 6.0, right = 8.0, bottom = 6.0, left = 8.0 }
[table.header]
font_weight = "bold"
text_color = "#E95420"
[table.cell]
text_color = "#1B1F23"
[table.border]
all = { width_pt = 0.75, color = "#D9D2CC", style = "solid" }

[admonition.note]
accent_color = "#E95420"
background_color = "#FDF0EC"
[admonition.info]
accent_color = "#77216F"
background_color = "#F5EDF4"
[admonition.tip]
accent_color = "#0E8420"
background_color = "#E9F5EB"
[admonition.warning]
accent_color = "#C7162B"
background_color = "#FCE9EA"
[admonition.danger]
accent_color = "#C7162B"
background_color = "#FCE9EA"
[admonition.generic]
accent_color = "#5E2750"
background_color = "#F0EDEF"
"##;

fn title_page_config_toml(info: &TitlePageInfo) -> String {
    format!(
        "{}\n[title_page]\ntitle = \"{}\"\nsubtitle = \"Prepared for {}\"\nauthor = \"{}\"\ndate = \"{}\"\n\n[title_page.style]\nfont_size_pt = 14.0\n",
        CANONICAL_THEME_CONFIG,
        toml_escape(&info.product),
        toml_escape(&info.company),
        toml_escape(&info.deployment_name),
        toml_escape(&info.generated_date),
    )
}

fn render_title_page(info: &TitlePageInfo) -> Result<Vec<u8>, String> {
    let config = title_page_config_toml(info);
    let bytes = render_markdown("", &config)?;
    let doc = Document::load_mem(&bytes).map_err(|e| e.to_string())?;
    let pages = doc.get_pages();
    let first_page_id = *pages.get(&1).ok_or("title page render produced no pages")?;

    let mut single = Document::with_version("1.5");
    single.objects = doc.objects.clone();
    single.max_id = doc.max_id;
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    let pages_id = single.new_object_id();
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = single.add_object(Object::Dictionary(catalog));
    single.trailer.set("Root", Object::Reference(catalog_id));

    let mut pages_dict = Dictionary::new();
    pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
    pages_dict.set("Count", Object::Integer(1));
    pages_dict.set("Kids", Object::Array(vec![Object::Reference(first_page_id)]));
    single.objects.insert(pages_id, Object::Dictionary(pages_dict));
    if let Ok(page_dict) = single.get_dictionary_mut(first_page_id) {
        page_dict.set("Parent", Object::Reference(pages_id));
    }

    stamp_logo(&mut single, first_page_id)?;

    let mut bytes_out = Vec::new();
    single.save_to(&mut bytes_out).map_err(|e| e.to_string())?;
    Ok(bytes_out)
}

fn build_toc_markdown(entries: &[(u8, String, u32)]) -> String {
    let mut out = String::from("# Table of Contents\n\n| Section | Page |\n| --- | ---: |\n");
    for (level, text, page) in entries {
        let escaped = text.replace('|', "\\|");
        if *level == 1 {
            out.push_str(&format!("| **{escaped}** | {page} |\n"));
        } else {
            out.push_str(&format!("| {escaped} | {page} |\n"));
        }
    }
    out
}

const BODY_CONFIG: &str = CANONICAL_THEME_CONFIG;

static DOT_DIAGRAM_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

const MODERN_DOT_STYLE: &str = r##"
  graph [fontname="Ubuntu,Helvetica,Arial,sans-serif", fontsize=11, bgcolor="transparent", pad="0.15"];
  node [shape=box, style="rounded,filled", fillcolor="#FDF3EF", color="#E95420", penwidth=1.3, fontname="Ubuntu,Helvetica,Arial,sans-serif", fontsize=11, fontcolor="#1B1F23", margin="0.18,0.10"];
  edge [color="#5E2750", penwidth=1.2, fontname="Ubuntu,Helvetica,Arial,sans-serif", fontsize=10, arrowsize=0.8];
"##;

const DOT_RENDER_DPI: u32 = 600;
const STANDALONE_DOT_SIZE: (f32, f32) = (6.3, 8.0);

fn apply_modern_dot_style(source: &str) -> String {
    match source.find('{') {
        Some(pos) => {
            let (head, tail) = source.split_at(pos + 1);
            format!("{head}{MODERN_DOT_STYLE}{tail}")
        }
        None => source.to_string(),
    }
}

fn render_one_dot_diagram(source: &str) -> Option<PathBuf> {
    let id = DOT_DIAGRAM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let base = format!("harvest-dot-diagram-{}-{id}", std::process::id());
    let src_path = dir.join(format!("{base}.dot"));
    let out_path = dir.join(format!("{base}.png"));
    let styled_source = apply_modern_dot_style(source);
    std::fs::write(&src_path, &styled_source).ok()?;

    let size_arg = format!("-Gsize={},{}", STANDALONE_DOT_SIZE.0, STANDALONE_DOT_SIZE.1);
    let status = std::process::Command::new("dot")
        .args(["-Tpng", &format!("-Gdpi={DOT_RENDER_DPI}"), &size_arg, "-o"])
        .arg(&out_path)
        .arg(&src_path)
        .status();

    let _ = std::fs::remove_file(&src_path);

    match status {
        Ok(s) if s.success() && out_path.exists() => Some(out_path),
        Ok(s) => {
            tracing::warn!(exit_code = ?s.code(), "dot diagram render failed, keeping raw source");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "dot binary unavailable, keeping raw source");
            None
        }
    }
}

fn render_dot_diagrams(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut lines = markdown.lines();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let fence_marker = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        let is_dot_fence = fence_marker
            .map(|marker| trimmed.trim_start_matches(marker).trim() == "dot")
            .unwrap_or(false);

        if !is_dot_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let fence_marker = fence_marker.unwrap();

        let mut block_lines: Vec<&str> = vec![line];
        let mut source = String::new();
        let mut closed = false;
        for content_line in lines.by_ref() {
            block_lines.push(content_line);
            if content_line.trim_start() == fence_marker {
                closed = true;
                break;
            }
            source.push_str(content_line);
            source.push('\n');
        }

        if closed {
            if let Some(path) = render_one_dot_diagram(&source) {
                out.push_str(&format!("![diagram]({})\n", path.display()));
                continue;
            }
        }

        for l in block_lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

pub fn build_design_pdf(markdown: &str, info: &TitlePageInfo) -> Result<Vec<u8>, String> {
    let markdown = render_dot_diagrams(markdown);
    let markdown = markdown.as_str();
    let body_bytes = render_markdown(markdown, BODY_CONFIG)?;
    let body_doc_for_outline = Document::load_mem(&body_bytes).map_err(|e| e.to_string())?;
    let entries = heading_pages(markdown, &body_doc_for_outline, 2);

    let toc_markdown = build_toc_markdown(&entries);
    let toc_bytes = render_markdown(&toc_markdown, BODY_CONFIG)?;

    let title_bytes = render_title_page(info)?;

    let title_doc = Document::load_mem(&title_bytes).map_err(|e| e.to_string())?;
    let toc_doc = Document::load_mem(&toc_bytes).map_err(|e| e.to_string())?;
    let body_doc = Document::load_mem(&body_bytes).map_err(|e| e.to_string())?;

    let title_page_count = title_doc.get_pages().len();
    let toc_page_count = toc_doc.get_pages().len();

    let mut merged = merge_documents(vec![title_doc, toc_doc, body_doc]);
    let page_ids: Vec<ObjectId> = merged.get_pages().into_values().collect();

    for (idx, page_id) in page_ids.iter().enumerate() {
        if idx < title_page_count {
            continue;
        } else if idx < title_page_count + toc_page_count {
            let label = lowercase_roman(idx - title_page_count + 1);
            stamp_page_label(&mut merged, *page_id, &label)?;
        } else {
            let label = (idx - title_page_count - toc_page_count + 1).to_string();
            stamp_page_label(&mut merged, *page_id, &label)?;
        }
    }

    let mut out = Vec::new();
    merged.save_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_roman_converts_small_numbers() {
        assert_eq!(lowercase_roman(1), "i");
        assert_eq!(lowercase_roman(4), "iv");
        assert_eq!(lowercase_roman(9), "ix");
        assert_eq!(lowercase_roman(14), "xiv");
        assert_eq!(lowercase_roman(40), "xl");
        assert_eq!(lowercase_roman(2024), "mmxxiv");
    }

    #[test]
    fn strip_inline_markdown_removes_emphasis_and_links() {
        assert_eq!(strip_inline_markdown("**Bold** and _italic_"), "Bold and italic");
        assert_eq!(strip_inline_markdown("[Link text](https://example.com)"), "Link text");
        assert_eq!(strip_inline_markdown("`code`"), "code");
    }

    #[test]
    fn parse_toc_headings_extracts_h1_and_h2_in_order() {
        let markdown = "# Intro\n\nBody.\n\n## Background\n\nMore.\n\n### Skip me\n\n# Architecture\n";
        let headings = parse_toc_headings(markdown, 2);
        assert_eq!(headings, vec![
            (1, "Intro".to_string()),
            (2, "Background".to_string()),
            (1, "Architecture".to_string()),
        ]);
    }

    #[test]
    fn parse_toc_headings_ignores_non_heading_hashes() {
        let markdown = "Not a heading #hashtag\n\n#NoSpace\n\n# Real Heading\n";
        let headings = parse_toc_headings(markdown, 2);
        assert_eq!(headings, vec![(1, "Real Heading".to_string())]);
    }

    #[test]
    fn parse_toc_headings_ignores_hashes_inside_fenced_code_blocks() {
        let markdown = "# Real Heading\n\n```text\n# fake heading\n## also fake\n```\n\n## Real Subheading\n";
        let headings = parse_toc_headings(markdown, 2);
        assert_eq!(headings, vec![
            (1, "Real Heading".to_string()),
            (2, "Real Subheading".to_string()),
        ]);
    }

    #[test]
    fn apply_modern_dot_style_injects_style_after_opening_brace_and_preserves_body() {
        let source = "digraph G {\n  a -> b;\n}\n";
        let styled = apply_modern_dot_style(source);
        assert!(styled.contains("style=\"rounded,filled\""));
        assert!(styled.contains("#E95420"));
        assert!(styled.contains("a -> b;"));
        assert!(styled.contains("fontname=\"Ubuntu,"), "diagram text should use the Ubuntu font");

        let brace_pos = styled.find('{').unwrap();
        let style_pos = styled.find("rounded,filled").unwrap();
        let body_pos = styled.find("a -> b;").unwrap();
        assert!(brace_pos < style_pos && style_pos < body_pos);
    }

    #[test]
    fn apply_modern_dot_style_lets_explicit_source_attributes_override_defaults() {
        let source = "digraph G {\n  node [shape=ellipse];\n  a -> b;\n}\n";
        let styled = apply_modern_dot_style(source);
        let default_pos = styled.find("shape=box").unwrap();
        let explicit_pos = styled.find("shape=ellipse").unwrap();
        assert!(default_pos < explicit_pos, "later attribute statements must win in dot");
    }

    fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
        if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            return None;
        }
        let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        Some((w, h))
    }

    #[test]
    fn render_one_dot_diagram_caps_output_size_for_large_graphs() {
        let mut source = String::from("digraph G {\n  rankdir=LR;\n");
        for i in 0..40 {
            source.push_str(&format!(
                "  \"Node number {i} with a fairly long label\" -> \"Node number {} with a fairly long label\";\n",
                i + 1
            ));
        }
        source.push_str("}\n");

        let path = render_one_dot_diagram(&source).expect("dot should render even a large graph");
        let bytes = std::fs::read(&path).expect("read png");
        let (w, h) = png_dimensions(&bytes).expect("valid png header");
        let dpi = DOT_RENDER_DPI as f32;
        assert!(w as f32 / dpi <= 6.35, "diagram wider than the page-safe cap: {w}px");
        assert!(h as f32 / dpi <= 8.05, "diagram taller than the page-safe cap: {h}px");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn render_dot_diagrams_replaces_dot_fence_with_image_reference() {
        let markdown = "Before.\n\n```dot\ndigraph G {\n  a -> b;\n}\n```\n\nAfter.\n";
        let out = render_dot_diagrams(markdown);
        assert!(!out.contains("```dot"), "dot fence should be replaced");
        assert!(!out.contains("digraph G"), "raw dot source should not remain");
        assert!(out.contains("![diagram]("), "expected an image reference");

        let path_start = out.find("![diagram](").unwrap() + "![diagram](".len();
        let path_end = out[path_start..].find(')').unwrap() + path_start;
        let path = &out[path_start..path_end];
        assert!(std::path::Path::new(path).exists(), "referenced image file must exist: {path}");
        assert!(path.ends_with(".png"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn render_dot_diagrams_leaves_non_dot_fences_untouched() {
        let markdown = "```text\nsome plain text\n```\n\n```\nunlabeled fence\n```\n";
        let out = render_dot_diagrams(markdown);
        assert_eq!(out, markdown);
    }

    #[test]
    fn render_dot_diagrams_falls_back_on_invalid_dot_source() {
        let markdown = "```dot\nthis is not { valid dot syntax\n```\n";
        let out = render_dot_diagrams(markdown);
        assert!(out.contains("```dot"), "invalid dot source should fall back to the raw fence");
        assert!(out.contains("this is not { valid dot syntax"));
    }

    #[test]
    fn render_dot_diagrams_handles_multiple_diagrams_independently() {
        let markdown = "```dot\ndigraph G {\n  a -> b;\n}\n```\n\nMiddle text.\n\n```dot\ndigraph H {\n  x -> y;\n}\n```\n";
        let out = render_dot_diagrams(markdown);
        let image_count = out.matches("![diagram](").count();
        assert_eq!(image_count, 2, "expected each dot block to render its own image");

        let mut paths = Vec::new();
        let mut rest = out.as_str();
        while let Some(start) = rest.find("![diagram](") {
            let after = &rest[start + "![diagram](".len()..];
            let end = after.find(')').unwrap();
            paths.push(after[..end].to_string());
            rest = &after[end..];
        }
        assert_eq!(paths.len(), 2);
        assert_ne!(paths[0], paths[1], "each diagram must get a distinct file");
        for p in &paths {
            assert!(std::path::Path::new(p).exists());
            let _ = std::fs::remove_file(p);
        }
    }

    fn render_plain(markdown: &str) -> Vec<u8> {
        render_markdown(markdown, BODY_CONFIG).expect("render")
    }

    #[test]
    fn title_page_config_toml_embeds_all_fields() {
        let info = TitlePageInfo {
            company: "Acme Corp".to_string(),
            product: "Landscape".to_string(),
            deployment_name: "Acme rollout".to_string(),
            generated_date: "2026-08-28".to_string(),
        };
        let toml = title_page_config_toml(&info);
        assert!(toml.contains("title = \"Landscape\""));
        assert!(toml.contains("Prepared for Acme Corp"));
        assert!(toml.contains("author = \"Acme rollout\""));
        assert!(toml.contains("date = \"2026-08-28\""));
        assert!(toml.contains("[title_page.style]"));
        assert!(toml.contains("font_size_pt"));
    }

    #[test]
    fn title_page_config_toml_escapes_quotes_and_backslashes() {
        let info = TitlePageInfo {
            company: "Quote \"Corp\"".to_string(),
            product: "Back\\slash".to_string(),
            deployment_name: "plain".to_string(),
            generated_date: "2026-08-28".to_string(),
        };
        let toml = title_page_config_toml(&info);
        assert!(toml.contains("Back\\\\slash"));
        assert!(toml.contains("Quote \\\"Corp\\\""));
    }

    #[test]
    fn canonical_theme_config_resolves_justified_paragraphs_and_styled_table() {
        use markdown2pdf::styling::{resolve, DocumentConfig, TextAlignment};

        let cfg: DocumentConfig = toml::from_str(CANONICAL_THEME_CONFIG).expect("parse theme toml");
        let resolved = resolve(cfg, None).expect("resolve theme");

        assert_eq!(resolved.paragraph.text_align, TextAlignment::Justify);
        assert!(resolved.table.border.top.is_some(), "expected a table border");
        assert!(
            resolved.table.alternating_row_background.is_some(),
            "expected zebra-striped table rows"
        );
        assert!(resolved.table.header.is_bold(), "expected a bold table header");
        assert_ne!(
            resolved.table.header.text_color, resolved.table.cell.text_color,
            "expected the header text color to stand out from body cells"
        );
        assert!(resolved.code_block.background_color.is_some(), "expected a code-block panel background");
        assert!(
            resolved.image.max_width_pct < 100.0,
            "expected diagrams to be capped below full column width"
        );
    }

    #[test]
    fn canonical_theme_config_resolves_black_headings() {
        use markdown2pdf::styling::{resolve, Color, DocumentConfig};

        let cfg: DocumentConfig = toml::from_str(CANONICAL_THEME_CONFIG).expect("parse theme toml");
        let resolved = resolve(cfg, None).expect("resolve theme");

        let black = Color::rgb(0x1B, 0x1F, 0x23);
        for (idx, heading) in resolved.headings.iter().take(3).enumerate() {
            assert_eq!(heading.text_color, black, "heading level {} should be black, not orange", idx + 1);
        }
    }

    #[test]
    fn heading_pages_maps_markdown_order_to_rendered_pages() {
        let markdown = "# Intro\n\nBody text.\n\n## Background\n\nMore body text.\n\n# Architecture\n\nFinal section.\n";
        let bytes = render_plain(markdown);
        let doc = Document::load_mem(&bytes).expect("load");
        let entries = heading_pages(markdown, &doc, 2);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].0, 1);
        assert_eq!(entries[0].1, "Intro");
        assert_eq!(entries[1].1, "Background");
        assert_eq!(entries[2].1, "Architecture");
        assert!(entries.iter().all(|(_, _, page)| *page >= 1));
    }

    #[test]
    fn merge_documents_concatenates_pages_from_each_source() {
        let a = render_plain("# Doc A\n\nContent A.");
        let b = render_plain("# Doc B\n\nContent B.");
        let doc_a = Document::load_mem(&a).expect("load a");
        let doc_b = Document::load_mem(&b).expect("load b");
        let a_pages = doc_a.get_pages().len();
        let b_pages = doc_b.get_pages().len();

        let mut merged = merge_documents(vec![doc_a, doc_b]);
        assert_eq!(merged.get_pages().len(), a_pages + b_pages);

        let mut out = Vec::new();
        merged.save_to(&mut out).expect("save");
        assert!(out.starts_with(b"%PDF-"));
        let reloaded = Document::load_mem(&out).expect("reload merged");
        assert_eq!(reloaded.get_pages().len(), a_pages + b_pages);
    }

    #[test]
    fn stamp_page_label_adds_visible_text_and_font_resource() {
        let bytes = render_plain("# Solo\n\nJust one page.");
        let mut doc = Document::load_mem(&bytes).expect("load");
        let page_id = *doc.get_pages().get(&1).unwrap();

        stamp_page_label(&mut doc, page_id, "iv").expect("stamp");

        let fonts = doc.get_page_fonts(page_id).expect("fonts");
        assert!(fonts.contains_key(PAGE_NUMBER_FONT_KEY));

        let content = doc.get_and_decode_page_content(page_id).expect("content");
        let has_label = content.operations.iter().any(|op| {
            op.operator == "Tj"
                && op.operands.first().and_then(|o| o.as_str().ok()) == Some(b"iv".as_slice())
        });
        assert!(has_label, "expected a Tj operation drawing \"iv\"");
    }

    #[test]
    fn stamp_page_label_preserves_existing_fonts_when_font_resource_is_a_reference() {
        let bytes = render_plain("# Solo\n\nJust one page.");
        let mut doc = Document::load_mem(&bytes).expect("load");
        let page_id = *doc.get_pages().get(&1).unwrap();

        let existing_font_id = doc.add_object(Object::Dictionary({
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"Font".to_vec()));
            d.set("Subtype", Object::Name(b"Type1".to_vec()));
            d.set("BaseFont", Object::Name(b"ExistingBodyFont".to_vec()));
            d
        }));
        let mut fonts_dict = Dictionary::new();
        fonts_dict.set("ExistingFontKey", Object::Reference(existing_font_id));
        let fonts_id = doc.add_object(Object::Dictionary(fonts_dict));
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Reference(fonts_id));
        let resources_id = doc.add_object(Object::Dictionary(resources));
        doc.get_dictionary_mut(page_id).unwrap().set("Resources", Object::Reference(resources_id));

        stamp_page_label(&mut doc, page_id, "iv").expect("stamp");

        let fonts = doc.get_page_fonts(page_id).expect("fonts");
        assert!(fonts.contains_key(b"ExistingFontKey".as_slice()), "existing font entry must survive");
        assert!(fonts.contains_key(PAGE_NUMBER_FONT_KEY), "new page-number font entry must be added");
    }

    #[test]
    fn stamp_logo_adds_xobject_resource_and_draw_operation() {
        let bytes = render_plain("# Solo\n\nJust one page.");
        let mut doc = Document::load_mem(&bytes).expect("load");
        let page_id = *doc.get_pages().get(&1).unwrap();

        stamp_logo(&mut doc, page_id).expect("stamp logo");

        let resources_id = page_resources_id(&mut doc, page_id).expect("resources id");
        let xobjects = doc.get_dictionary(resources_id).expect("resources dict").get(b"XObject").expect("XObject entry");
        let xobjects_dict = match xobjects {
            Object::Reference(id) => doc.get_dictionary(*id).expect("xobject dict"),
            Object::Dictionary(d) => d,
            _ => panic!("unexpected XObject entry type"),
        };
        assert!(xobjects_dict.has(UBUNTU_LOGO_KEY), "expected an XObject resource entry for the logo");

        let content = doc.get_and_decode_page_content(page_id).expect("content");
        let has_draw = content.operations.iter().any(|op| {
            op.operator == "Do"
                && op.operands.first().and_then(|o| o.as_name().ok()) == Some(UBUNTU_LOGO_KEY)
        });
        assert!(has_draw, "expected a Do operation drawing the logo");
    }

    #[test]
    fn stamp_logo_preserves_source_aspect_ratio() {
        assert_eq!(
            (UBUNTU_LOGO_WIDTH_PX * UBUNTU_LOGO_HEIGHT_PX * 3) as usize,
            UBUNTU_LOGO_RGB.len(),
            "image dict dimensions must match the embedded raw RGB asset's actual size"
        );

        let bytes = render_plain("# Solo\n\nJust one page.");
        let mut doc = Document::load_mem(&bytes).expect("load");
        let page_id = *doc.get_pages().get(&1).unwrap();

        stamp_logo(&mut doc, page_id).expect("stamp logo");

        let content = doc.get_and_decode_page_content(page_id).expect("content");
        let cm = content
            .operations
            .iter()
            .find(|op| op.operator == "cm")
            .expect("expected a cm operation scaling the logo");
        let width = cm.operands[0].as_float().expect("width operand");
        let height = cm.operands[3].as_float().expect("height operand");
        let expected_ratio = UBUNTU_LOGO_HEIGHT_PX as f32 / UBUNTU_LOGO_WIDTH_PX as f32;
        let actual_ratio = height / width;
        assert!(
            (actual_ratio - expected_ratio).abs() < 0.01,
            "expected drawn logo to preserve source aspect ratio: got {actual_ratio}, want {expected_ratio}"
        );
    }

    fn minimal_page_with_content_ending_without_whitespace() -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.5");
        let content_id = doc.add_object(Stream::new(Dictionary::new(), b"q 1 0 0 1 0 0 cm Q".to_vec()));
        let mut page_dict = Dictionary::new();
        page_dict.set("Type", Object::Name(b"Page".to_vec()));
        page_dict.set(
            "MediaBox",
            Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(595.0), Object::Real(842.0)]),
        );
        page_dict.set("Resources", Object::Dictionary(Dictionary::new()));
        page_dict.set("Contents", Object::Reference(content_id));
        let page_id = doc.add_object(Object::Dictionary(page_dict));

        let mut pages_dict = Dictionary::new();
        pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
        pages_dict.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        pages_dict.set("Count", Object::Integer(1));
        let pages_id = doc.add_object(Object::Dictionary(pages_dict));
        doc.get_dictionary_mut(page_id).unwrap().set("Parent", Object::Reference(pages_id));

        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));

        (doc, page_id)
    }

    #[test]
    fn stamp_logo_separates_from_preceding_content_with_no_trailing_whitespace() {
        let (mut doc, page_id) = minimal_page_with_content_ending_without_whitespace();

        stamp_logo(&mut doc, page_id).expect("stamp logo");

        let content = doc.get_and_decode_page_content(page_id).expect("content");
        assert!(
            content.operations.iter().all(|op| op.operator != "Qq"),
            "content streams were concatenated without a separator"
        );
        let has_do = content.operations.iter().any(|op| {
            op.operator == "Do" && op.operands.first().and_then(|o| o.as_name().ok()) == Some(UBUNTU_LOGO_KEY)
        });
        assert!(has_do, "expected a properly parsed Do operation drawing the logo");
    }

    #[test]
    fn stamp_page_label_separates_from_preceding_content_with_no_trailing_whitespace() {
        let (mut doc, page_id) = minimal_page_with_content_ending_without_whitespace();

        stamp_page_label(&mut doc, page_id, "3").expect("stamp label");

        let content = doc.get_and_decode_page_content(page_id).expect("content");
        assert!(
            content.operations.iter().all(|op| op.operator != "Qq"),
            "content streams were concatenated without a separator"
        );
        let has_label = content.operations.iter().any(|op| {
            op.operator == "Tj" && op.operands.first().and_then(|o| o.as_str().ok()) == Some(b"3".as_slice())
        });
        assert!(has_label, "expected a properly parsed Tj operation drawing the label");
    }

    #[test]
    fn build_design_pdf_produces_title_toc_and_numbered_body_pages() {
        let markdown = "# Intro\n\nBody text one.\n\n## Background\n\nMore text.\n\n# Architecture\n\nFinal section content.\n";
        let info = TitlePageInfo {
            company: "Acme Corp".to_string(),
            product: "Landscape".to_string(),
            deployment_name: "Acme rollout".to_string(),
            generated_date: "2026-08-28".to_string(),
        };

        let body_only = render_markdown(markdown, BODY_CONFIG).expect("render body only");
        let body_doc = Document::load_mem(&body_only).expect("load body");
        let body_page_count = body_doc.get_pages().len();

        let pdf_bytes = build_design_pdf(markdown, &info).expect("build design pdf");
        assert!(pdf_bytes.starts_with(b"%PDF-"));

        let doc = Document::load_mem(&pdf_bytes).expect("load merged");
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 1 + 1 + body_page_count);

        let title_page_id = *pages.get(&1).unwrap();
        let title_content = doc.get_and_decode_page_content(title_page_id).expect("title content");
        assert!(!title_content.operations.iter().any(|op| op.operator == "Tj"
            && op.operands.first().and_then(|o| o.as_str().ok()).map(|s| s == b"i".as_slice()).unwrap_or(false)));

        let toc_page_id = *pages.get(&2).unwrap();
        let toc_content = doc.get_and_decode_page_content(toc_page_id).expect("toc content");
        let has_roman_i = toc_content.operations.iter().any(|op| {
            op.operator == "Tj" && op.operands.first().and_then(|o| o.as_str().ok()) == Some(b"i".as_slice())
        });
        assert!(has_roman_i);

        let first_body_page_id = *pages.get(&3).unwrap();
        let body_content = doc.get_and_decode_page_content(first_body_page_id).expect("body content");
        let has_arabic_1 = body_content.operations.iter().any(|op| {
            op.operator == "Tj" && op.operands.first().and_then(|o| o.as_str().ok()) == Some(b"1".as_slice())
        });
        assert!(has_arabic_1);
    }
}







