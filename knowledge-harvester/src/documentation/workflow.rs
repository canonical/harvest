use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use super::llm::LlmClient;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Feature {
    pub name: String,
    pub description: String,
    pub related_files: Vec<String>,
    pub related_symbols: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DescribedFeature {
    pub name: String,
    pub brief: String,
    pub description: String,
    pub intent: String,
    pub related_files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocPage {
    pub filename: String,
    pub title: String,
    pub content: String,
}

/// Lightweight plan returned by Phase 4a — filename and title only, no content.
#[derive(Debug, Deserialize)]
struct PagePlan {
    filename: String,
    title: String,
}

pub struct StructureRow {
    pub path: String,
    pub language: String,
    pub symbols: Vec<SymbolInfo>,
}

pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
}

pub struct Workflow<'a> {
    pub llm: &'a dyn LlmClient,
    pub docs_dir: &'a PathBuf,
}

impl<'a> Workflow<'a> {
    /// Phase 1: Identify major features from codebase structure.
    pub async fn identify_features(
        &self,
        repo: &str,
        version: &str,
        structure: &[StructureRow],
    ) -> Result<Vec<Feature>> {
        let structure_text = format_structure(structure);
        let system = "You are a technical documentation expert. \
            Your task is to analyse a codebase structure and identify its major features and components. \
            Always respond with valid JSON only — no prose, no markdown fences.";
        let user = format!(
            "Repository: {repo}, version: {version}\n\n\
            Codebase structure (files and symbols):\n{structure_text}\n\n\
            Identify 3 to 8 major features or components of this codebase.\n\
            Respond with a JSON array. Each element must have:\n\
            - \"name\": short descriptive feature name\n\
            - \"description\": one-sentence description\n\
            - \"related_files\": array of up to 5 most relevant file paths\n\
            - \"related_symbols\": array of up to 10 most relevant function or class names\n\
            Example: [{{\"name\":\"Auth\",\"description\":\"Handles user auth\",\
            \"related_files\":[\"src/auth.rs\"],\"related_symbols\":[\"authenticate\"]}}]"
        );
        let raw = self.llm.complete(system, &user).await?;
        parse_json_array(&raw, "features")
    }

    /// Phase 2+3: For each feature, generate detailed description and intent
    /// using the full source code of related symbols.
    pub async fn describe_features(
        &self,
        repo: &str,
        version: &str,
        features: &[Feature],
        sources: &[(String, String)],
    ) -> Result<Vec<DescribedFeature>> {
        let mut described = Vec::new();
        for feature in features {
            let relevant_sources = sources
                .iter()
                .filter(|(name, _)| feature.related_symbols.iter().any(|s| s == name))
                .map(|(name, src)| format!("### {name}\n```\n{src}\n```"))
                .collect::<Vec<_>>()
                .join("\n\n");

            let system = "You are a technical documentation expert. \
                Analyse the provided source code and produce structured descriptions. \
                Always respond with valid JSON only — no prose, no markdown fences.";
            let user = format!(
                "Repository: {repo}, version: {version}\n\
                Feature: {}\nBrief description: {}\n\n\
                Source code of related symbols:\n{relevant_sources}\n\n\
                Respond with a JSON object with two fields:\n\
                - \"description\": detailed technical description (2-4 paragraphs) of what this feature does\n\
                - \"intent\": explanation of purpose, who uses this and why (1-2 paragraphs)",
                feature.name, feature.description
            );
            let raw = self.llm.complete(system, &user).await?;
            let obj: Value = parse_json_object(&raw, "feature description")?;
            described.push(DescribedFeature {
                name: feature.name.clone(),
                brief: feature.description.clone(),
                description: obj["description"].as_str().unwrap_or("").to_string(),
                intent: obj["intent"].as_str().unwrap_or("").to_string(),
                related_files: feature.related_files.clone(),
            });
        }
        Ok(described)
    }

    /// Phase 4a: Ask the LLM for a page plan (filename + title only) for one section.
    /// Returns a small JSON array — well within token limits.
    async fn plan_section_pages(
        &self,
        repo: &str,
        version: &str,
        section: &str,
        section_guidance: &str,
        features: &[DescribedFeature],
    ) -> Result<Vec<PagePlan>> {
        let features_summary = features
            .iter()
            .map(|f| format!("- {}: {}", f.name, f.brief))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "You are a technical documentation expert following the Diataxis framework \
            (https://diataxis.fr/). \
            Always respond with valid JSON only — no prose, no markdown fences.";
        let user = format!(
            "Repository: {repo}, version: {version}\n\
            Documentation section: {section}\n{section_guidance}\n\n\
            Features in this codebase:\n{features_summary}\n\n\
            Plan 1 to 4 documentation pages for the \"{section}\" section.\n\
            Respond with a JSON array. Each element must have exactly two fields:\n\
            - \"filename\": kebab-case filename ending in .md (e.g. \"getting-started.md\")\n\
            - \"title\": human-readable page title\n\
            Example: [{{\"filename\":\"getting-started.md\",\"title\":\"Getting Started\"}}]"
        );
        let raw = self.llm.complete(system, &user).await?;
        parse_json_array(&raw, "page plan")
    }

    /// Phase 4b: Generate the content of one page as raw markdown.
    /// No JSON wrapping — the response is written to disk directly, so there
    /// is no risk of the output being truncated mid-string inside a JSON value.
    async fn generate_page_content(
        &self,
        repo: &str,
        version: &str,
        section: &str,
        section_guidance: &str,
        page: &PagePlan,
        features: &[DescribedFeature],
        sources: &[(String, String)],
    ) -> Result<String> {
        let features_text = features
            .iter()
            .map(|f| format!("## {}\nDescription: {}\nIntent: {}", f.name, f.description, f.intent))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let source_snippets = sources
            .iter()
            .take(15)
            .map(|(name, src)| format!("### {name}\n```\n{}\n```", truncate(src, 400)))
            .collect::<Vec<_>>()
            .join("\n\n");

        let system = format!(
            "You are a technical documentation expert following the Diataxis framework \
            (https://diataxis.fr/). \
            Write clear, well-structured documentation in markdown. \
            {section_guidance}"
        );
        let user = format!(
            "Repository: {repo}, version: {version}\n\
            Documentation section: {section}\n\
            Page title: {}\n\
            Page filename: {}\n\n\
            Features and descriptions:\n{features_text}\n\n\
            Source code snippets for reference:\n{source_snippets}\n\n\
            Write the full markdown content for this page. \
            Begin with a # heading matching the page title. \
            Use code examples, headings, and lists as appropriate. \
            Respond with markdown only — do not wrap the response in JSON or code fences.",
            page.title, page.filename
        );
        self.llm.complete(&system, &user).await
    }

    /// Phase 4: Generate all pages for one Diataxis section.
    /// Uses two LLM calls per page (plan then content) so each call stays
    /// well within the output token limit regardless of page length.
    pub async fn generate_section(
        &self,
        repo: &str,
        version: &str,
        section: &str,
        section_guidance: &str,
        features: &[DescribedFeature],
        sources: &[(String, String)],
    ) -> Result<Vec<DocPage>> {
        let plans = self
            .plan_section_pages(repo, version, section, section_guidance, features)
            .await?;

        let mut pages = Vec::new();
        for plan in &plans {
            tracing::info!(section, filename = plan.filename, "generating page");
            let content = self
                .generate_page_content(
                    repo, version, section, section_guidance, plan, features, sources,
                )
                .await?;
            pages.push(DocPage {
                filename: plan.filename.clone(),
                title: plan.title.clone(),
                content,
            });
        }
        Ok(pages)
    }

    /// Run the full 4-phase workflow for a repo/version.
    pub async fn run(
        &self,
        repo: &str,
        version: &str,
        structure: &[StructureRow],
        sources: &[(String, String)],
    ) -> Result<()> {
        tracing::info!(repo, version, "Phase 1: identifying features");
        let features = self.identify_features(repo, version, structure).await?;
        tracing::info!(repo, version, count = features.len(), "identified features");

        tracing::info!(repo, version, "Phase 2+3: describing features and intent");
        let described = self.describe_features(repo, version, &features, sources).await?;

        tracing::info!(repo, version, "Phase 4: generating Diataxis documentation");
        let sections = [
            ("tutorials",     "Tutorials are learning-oriented. They guide a newcomer through a hands-on exercise that teaches them how to use the software by doing, not by explaining. Focus on getting someone started successfully."),
            ("how-to-guides", "How-to guides are goal-oriented. They show the reader how to solve a specific, real-world problem. Assume the reader has some competence and wants to accomplish a task."),
            ("explanations",  "Explanations are understanding-oriented. They illuminate a topic from multiple angles and provide background, context, and reasoning. Help the reader understand *why* things are the way they are."),
            ("reference",     "Reference material is information-oriented. It provides accurate, complete, factual descriptions of the API, configuration, commands, and data structures. Dry, precise, structured."),
        ];

        let out_base = self.docs_dir.join(repo).join(version);
        let mut index_sections = std::collections::HashMap::new();

        for (section, guidance) in &sections {
            tracing::info!(repo, version, section, "generating section");
            let pages = self
                .generate_section(repo, version, section, guidance, &described, sources)
                .await?;

            let section_dir = out_base.join(section);
            tokio::fs::create_dir_all(&section_dir).await
                .with_context(|| format!("creating section dir {}", section_dir.display()))?;

            let mut entries = Vec::new();
            for page in &pages {
                let file_path = section_dir.join(&page.filename);
                tokio::fs::write(&file_path, &page.content).await
                    .with_context(|| format!("writing {}", file_path.display()))?;
                entries.push(crate::documentation::IndexEntry {
                    filename: page.filename.clone(),
                    title: page.title.clone(),
                });
                tracing::info!(section, filename = page.filename, "wrote doc page");
            }
            index_sections.insert(section.to_string(), entries);
        }

        let index = crate::documentation::DocIndex {
            repo: repo.to_string(),
            version: version.to_string(),
            generated_at: chrono_now(),
            sections: index_sections,
        };
        let index_json = serde_json::to_string_pretty(&index)?;
        tokio::fs::write(out_base.join("index.json"), index_json).await?;

        tracing::info!(repo, version, "documentation generated successfully");
        Ok(())
    }
}

fn format_structure(rows: &[StructureRow]) -> String {
    rows.iter()
        .map(|row| {
            let symbols = row
                .symbols
                .iter()
                .map(|s| {
                    if let Some(sig) = &s.signature {
                        format!("  {} {} ({})", s.kind, s.name, sig)
                    } else {
                        format!("  {} {}", s.kind, s.name)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            if symbols.is_empty() {
                format!("{} [{}]", row.path, row.language)
            } else {
                format!("{} [{}]\n{symbols}", row.path, row.language)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_json_array<T: serde::de::DeserializeOwned>(raw: &str, context: &str) -> Result<Vec<T>> {
    let trimmed = extract_json(raw);
    serde_json::from_str(trimmed)
        .with_context(|| format!("parsing {context} JSON array from LLM response: {raw}"))
}

fn parse_json_object(raw: &str, context: &str) -> Result<Value> {
    let trimmed = extract_json(raw);
    serde_json::from_str(trimmed)
        .with_context(|| format!("parsing {context} JSON object from LLM response: {raw}"))
}

/// Strip markdown code fences and extract raw JSON from LLM output.
fn extract_json(raw: &str) -> &str {
    let s = raw.trim();
    if s.starts_with("```") {
        let after_fence = s.find('\n').map(|i| &s[i + 1..]).unwrap_or(s);
        after_fence.trim_end_matches("```").trim_end_matches("```json").trim()
    } else {
        s
    }
}

fn truncate(s: &str, max_chars: usize) -> &str {
    let mut end = s.len().min(max_chars);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi, s) = epoch_to_datetime(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn epoch_to_datetime(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mi = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    let y = days / 365 + 1970;
    let yday = days % 365;
    let mo = yday / 30 + 1;
    let d = yday % 30 + 1;
    (y, mo.min(12), d.min(31), h, mi, s)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::llm::MockLlmClient;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    // ── helpers ───────────────────────────────────────────────────────────────

    fn make_workflow<'a>(llm: &'a dyn LlmClient, docs_dir: &'a PathBuf) -> Workflow<'a> {
        Workflow { llm, docs_dir }
    }

    fn sample_structure() -> Vec<StructureRow> {
        vec![
            StructureRow {
                path: "src/main.rs".to_string(),
                language: "rust".to_string(),
                symbols: vec![SymbolInfo {
                    name: "main".to_string(),
                    kind: "Function".to_string(),
                    signature: Some("fn main()".to_string()),
                }],
            },
            StructureRow {
                path: "src/auth.rs".to_string(),
                language: "rust".to_string(),
                symbols: vec![
                    SymbolInfo {
                        name: "authenticate".to_string(),
                        kind: "Function".to_string(),
                        signature: Some("fn authenticate(token: &str) -> bool".to_string()),
                    },
                    SymbolInfo {
                        name: "AuthManager".to_string(),
                        kind: "Class".to_string(),
                        signature: None,
                    },
                ],
            },
        ]
    }

    fn features_json() -> String {
        serde_json::to_string(&vec![Feature {
            name: "Authentication".to_string(),
            description: "Handles user authentication".to_string(),
            related_files: vec!["src/auth.rs".to_string()],
            related_symbols: vec!["authenticate".to_string(), "AuthManager".to_string()],
        }])
        .unwrap()
    }

    fn describe_json() -> String {
        serde_json::json!({
            "description": "The auth module handles user authentication via tokens.",
            "intent": "Provides secure access control for the application."
        })
        .to_string()
    }

    /// A page-plan response: just filename + title, no content.
    fn plan_json() -> String {
        serde_json::json!([{"filename": "getting-started.md", "title": "Getting Started"}])
            .to_string()
    }

    /// Raw markdown content (no JSON wrapper) returned by Phase 4b.
    fn page_markdown() -> String {
        "# Getting Started\n\nLearn to use the auth module.".to_string()
    }

    fn sample_described() -> Vec<DescribedFeature> {
        vec![DescribedFeature {
            name: "Auth".to_string(),
            brief: "Auth module".to_string(),
            description: "Handles authentication.".to_string(),
            intent: "Secure access control.".to_string(),
            related_files: vec![],
        }]
    }

    /// A mock that returns a different response per call index.
    struct SequenceMock {
        calls: Arc<AtomicUsize>,
        responses: Vec<String>,
    }

    impl SequenceMock {
        fn new(responses: Vec<String>) -> Self {
            Self { calls: Arc::new(AtomicUsize::new(0)), responses }
        }
    }

    #[async_trait]
    impl LlmClient for SequenceMock {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            let idx = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.responses[idx % self.responses.len()].clone())
        }
    }

    // ── Phase 1 ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn identify_features_parses_llm_json() {
        let llm = MockLlmClient { response: features_json() };
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        let feats = make_workflow(&llm, &docs_dir)
            .identify_features("myrepo", "v1.0", &sample_structure())
            .await
            .unwrap();
        assert_eq!(feats.len(), 1);
        assert_eq!(feats[0].name, "Authentication");
        assert_eq!(feats[0].related_symbols, ["authenticate", "AuthManager"]);
    }

    #[tokio::test]
    async fn identify_features_strips_json_fences() {
        let fenced = format!("```json\n{}\n```", features_json());
        let llm = MockLlmClient { response: fenced };
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        let feats = make_workflow(&llm, &docs_dir)
            .identify_features("myrepo", "v1.0", &sample_structure())
            .await
            .unwrap();
        assert_eq!(feats.len(), 1);
    }

    // ── Phase 2+3 ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn describe_features_produces_described_features() {
        let llm = MockLlmClient { response: describe_json() };
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        let features = serde_json::from_str::<Vec<Feature>>(&features_json()).unwrap();
        let sources = vec![
            ("authenticate".to_string(), "fn authenticate(t: &str) -> bool { true }".to_string()),
        ];
        let described = make_workflow(&llm, &docs_dir)
            .describe_features("myrepo", "v1.0", &features, &sources)
            .await
            .unwrap();
        assert_eq!(described.len(), 1);
        assert_eq!(described[0].name, "Authentication");
        assert!(!described[0].description.is_empty());
        assert!(!described[0].intent.is_empty());
    }

    // ── Phase 4 ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn generate_section_returns_doc_pages() {
        // Two LLM calls per section: plan (JSON) then content (raw markdown).
        let llm = SequenceMock::new(vec![plan_json(), page_markdown()]);
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        let pages = make_workflow(&llm, &docs_dir)
            .generate_section("repo", "v1.0", "tutorials", "tutorial guidance", &sample_described(), &[])
            .await
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].filename, "getting-started.md");
        assert_eq!(pages[0].title, "Getting Started");
        assert!(pages[0].content.contains("# Getting Started"));
    }

    #[tokio::test]
    async fn generate_section_content_is_raw_markdown_not_json() {
        // Verify that page content is stored as-is from the LLM, not JSON-parsed.
        // A truncated JSON string would fail JSON parsing but succeed here.
        let raw_md = "# My Page\n\nSome content with a `code` snippet.\n\n## Section\n\nMore.";
        let llm = SequenceMock::new(vec![plan_json(), raw_md.to_string()]);
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        let pages = make_workflow(&llm, &docs_dir)
            .generate_section("repo", "v1.0", "tutorials", "guidance", &sample_described(), &[])
            .await
            .unwrap();
        assert_eq!(pages[0].content, raw_md);
    }

    #[tokio::test]
    async fn generate_section_makes_two_llm_calls_per_page() {
        // Exactly 2 calls for 1 planned page: plan call + content call.
        let counter = Arc::new(AtomicUsize::new(0));
        struct CountingMock(Arc<AtomicUsize>, Vec<String>);
        #[async_trait]
        impl LlmClient for CountingMock {
            async fn complete(&self, _s: &str, _u: &str) -> Result<String> {
                let idx = self.0.fetch_add(1, Ordering::SeqCst);
                Ok(self.1[idx % self.1.len()].clone())
            }
        }
        let llm = CountingMock(Arc::clone(&counter), vec![plan_json(), page_markdown()]);
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        make_workflow(&llm, &docs_dir)
            .generate_section("repo", "v1.0", "tutorials", "guidance", &sample_described(), &[])
            .await
            .unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 2, "expected exactly 2 LLM calls");
    }

    #[tokio::test]
    async fn run_writes_all_section_dirs_and_index() {
        // Phase 1:    1 call  → features_json
        // Phase 2+3:  1 call  → describe_json  (one feature)
        // Phase 4:    per section: plan call + content call = 2 × 4 = 8 calls
        // Total:     10 calls
        let responses = vec![
            features_json(),   // phase 1
            describe_json(),   // phase 2+3
            plan_json(),       // tutorials  — plan
            page_markdown(),   // tutorials  — content
            plan_json(),       // how-to-guides — plan
            page_markdown(),   // how-to-guides — content
            plan_json(),       // explanations  — plan
            page_markdown(),   // explanations  — content
            plan_json(),       // reference     — plan
            page_markdown(),   // reference     — content
        ];
        let llm = SequenceMock::new(responses);
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        let workflow = Workflow { llm: &llm, docs_dir: &docs_dir };

        workflow
            .run("testrepo", "v1.0", &sample_structure(), &[])
            .await
            .unwrap();

        for section in &["tutorials", "how-to-guides", "explanations", "reference"] {
            let section_dir = dir.path().join("testrepo").join("v1.0").join(section);
            assert!(section_dir.exists(), "missing section dir: {section}");
            // The single page file should exist and contain raw markdown.
            let page = section_dir.join("getting-started.md");
            assert!(page.exists(), "missing page file in {section}");
            let content = std::fs::read_to_string(&page).unwrap();
            assert!(content.starts_with("# Getting Started"), "page content in {section}");
        }

        let index_path = dir.path().join("testrepo").join("v1.0").join("index.json");
        assert!(index_path.exists(), "missing index.json");
        let index: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap()).unwrap();
        assert_eq!(index["repo"], "testrepo");
        assert_eq!(index["version"], "v1.0");
        for section in &["tutorials", "how-to-guides", "explanations", "reference"] {
            assert!(
                index["sections"][section].as_array().is_some(),
                "missing section in index: {section}"
            );
        }
    }

    #[tokio::test]
    async fn run_total_llm_call_count_matches_expected() {
        // Verify the exact number of LLM calls for a known input.
        // 1 feature → 1 + 1 + (2 × 4) = 10 calls.
        let counter = Arc::new(AtomicUsize::new(0));
        struct CountingMock(Arc<AtomicUsize>);
        #[async_trait]
        impl LlmClient for CountingMock {
            async fn complete(&self, _s: &str, _u: &str) -> Result<String> {
                let idx = self.0.fetch_add(1, Ordering::SeqCst);
                let responses = vec![
                    features_json(), describe_json(),
                    plan_json(), page_markdown(),
                    plan_json(), page_markdown(),
                    plan_json(), page_markdown(),
                    plan_json(), page_markdown(),
                ];
                Ok(responses[idx % responses.len()].clone())
            }
        }
        let llm = CountingMock(Arc::clone(&counter));
        let dir = tempfile::tempdir().unwrap();
        let docs_dir = dir.path().to_path_buf();
        Workflow { llm: &llm, docs_dir: &docs_dir }
            .run("testrepo", "v1.0", &sample_structure(), &[])
            .await
            .unwrap();
        assert_eq!(
            counter.load(Ordering::SeqCst), 10,
            "expected 10 LLM calls for 1 feature and 4 sections with 1 page each"
        );
    }
}
