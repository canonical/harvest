// Throwaway helper: render a markdown file to a design-doc PDF using the
// same renderer the server uses, for manual inspection outside the app.
// Usage: cargo run --example render_design_pdf -- <input.md> <output.pdf> [title]

use knowledge_server::deployments::design_pdf::{build_design_pdf, TitlePageInfo};
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = args.get(1).expect("usage: render_design_pdf <input.md> <output.pdf> [title]");
    let output = args.get(2).expect("usage: render_design_pdf <input.md> <output.pdf> [title]");
    let title = args.get(3).cloned().unwrap_or_else(|| "Design".to_string());

    let markdown = fs::read_to_string(input).expect("failed to read input markdown");
    let info = TitlePageInfo {
        company: String::new(),
        product: title.clone(),
        deployment_name: title,
        generated_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
    };
    let pdf = build_design_pdf(&markdown, &info).expect("pdf render failed");
    fs::write(output, pdf).expect("failed to write output pdf");
    println!("wrote {output}");
}
