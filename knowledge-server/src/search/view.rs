use harvest_search::domain::UnitId;
use harvest_search::index::corpus::{Relation, UnitKind};
use harvest_search::index::graph::Direction;
use harvest_search::index::IndexSnapshot;
use harvest_search::{Evidence, Query, Scope, SearchResult};
use serde::Serialize;

pub const VIEW_KIND: &str = "code_search";
pub const MAX_ITEM_CODE_CHARS: usize = 6_000;
pub const MAX_EXCERPT_CHARS: usize = 1_200;
pub const MAX_LINKS: usize = 12;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Location {
    pub name: String,
    pub file: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodeItem {
    pub kind: String,
    pub name: String,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub code: String,
    pub truncated: bool,
    pub owner: Option<Location>,
    pub calls: Vec<Location>,
    pub rows: Vec<TableRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TableRow {
    pub symbol: Option<Location>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DocumentationItem {
    pub name: String,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub excerpt: String,
    pub points_to: Vec<Location>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchView {
    pub kind: &'static str,
    pub repo: String,
    pub version: String,
    pub query: Option<String>,
    pub focus: Vec<String>,
    pub tier: Option<String>,
    pub total_ms: Option<f64>,
    pub degraded: Option<String>,
    pub items: Vec<CodeItem>,
    pub documentation: Vec<DocumentationItem>,
    pub gaps: Vec<String>,
}

fn clip(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

fn table_row(line: &str) -> TableRow {
    let symbol_end = line.find(" (").and_then(|open| {
        let close = open + line[open..].find(')')?;
        let (file, number) = line[open + 2..close].rsplit_once(':')?;
        let symbol = Location {
            name: line[..open].to_string(),
            file: file.to_string(),
            line: number.parse().ok()?,
        };
        Some((symbol, close + 1))
    });
    match symbol_end {
        Some((symbol, end)) => TableRow {
            symbol: Some(symbol),
            detail: line[end..].trim_start_matches(':').trim().to_string(),
        },
        None => TableRow {
            symbol: None,
            detail: line.trim().to_string(),
        },
    }
}

fn table_rows(text: &str) -> Vec<TableRow> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(table_row)
        .collect()
}

fn location(index: &IndexSnapshot, unit: UnitId) -> Location {
    let unit = index.unit(unit);
    Location {
        name: unit.qualified_name.clone(),
        file: unit.file.clone(),
        line: unit.start_line,
    }
}

fn linked(
    index: &IndexSnapshot,
    unit: UnitId,
    direction: Direction,
    relation: Relation,
) -> Vec<UnitId> {
    let mut found: Vec<UnitId> = Vec::new();
    for target in index.graph.neighbors(unit, direction, &[relation]) {
        if target != unit && !found.contains(&target) {
            found.push(target);
        }
    }
    found
}

fn calls_of(index: &IndexSnapshot, unit: UnitId) -> Vec<Location> {
    linked(index, unit, Direction::Out, Relation::Calls)
        .into_iter()
        .take(MAX_LINKS)
        .map(|target| location(index, target))
        .collect()
}

fn owner_of(index: &IndexSnapshot, unit: UnitId) -> Option<Location> {
    linked(index, unit, Direction::In, Relation::Contains)
        .into_iter()
        .find(|owner| index.unit(*owner).kind() == UnitKind::Class)
        .map(|owner| location(index, owner))
}

fn points_to(index: &IndexSnapshot, unit: UnitId) -> Vec<Location> {
    linked(index, unit, Direction::Out, Relation::Documents)
        .into_iter()
        .filter(|target| !index.unit(*target).kind().is_documentation())
        .take(MAX_LINKS)
        .map(|target| location(index, target))
        .collect()
}

fn kind_from_label(label: &str) -> String {
    label
        .split_whitespace()
        .next()
        .unwrap_or("evidence")
        .to_string()
}

fn code_item(index: Option<&IndexSnapshot>, evidence: &Evidence) -> CodeItem {
    match (index, evidence.unit) {
        (Some(index), Some(id)) => {
            let unit = index.unit(id);
            let source = if unit.body.is_empty() {
                &evidence.text
            } else {
                &unit.body
            };
            let (code, truncated) = clip(source, MAX_ITEM_CODE_CHARS);
            CodeItem {
                kind: unit.kind().as_str().to_string(),
                name: unit.qualified_name.clone(),
                file: unit.file.clone(),
                start_line: unit.start_line,
                end_line: unit.end_line,
                code,
                truncated,
                owner: owner_of(index, id),
                calls: calls_of(index, id),
                rows: Vec::new(),
            }
        }
        _ => {
            let (code, truncated) = clip(&evidence.text, MAX_ITEM_CODE_CHARS);
            let table = evidence.unit.is_none() && evidence.key.file().is_empty();
            CodeItem {
                kind: if table {
                    "table".to_string()
                } else {
                    kind_from_label(&evidence.label)
                },
                name: evidence.label.clone(),
                file: evidence.key.file().to_string(),
                start_line: evidence.start_line,
                end_line: evidence.end_line,
                code,
                truncated,
                owner: None,
                calls: Vec::new(),
                rows: if table {
                    table_rows(&evidence.text)
                } else {
                    Vec::new()
                },
            }
        }
    }
}

fn documentation_item(index: Option<&IndexSnapshot>, evidence: &Evidence) -> DocumentationItem {
    let (name, source, links) = match (index, evidence.unit) {
        (Some(index), Some(id)) => {
            let unit = index.unit(id);
            (unit.name.clone(), unit.body.clone(), points_to(index, id))
        }
        _ => (evidence.label.clone(), evidence.text.clone(), Vec::new()),
    };
    DocumentationItem {
        name,
        file: evidence.key.file().to_string(),
        start_line: evidence.start_line,
        end_line: evidence.end_line,
        excerpt: clip(source.trim(), MAX_EXCERPT_CHARS).0,
        points_to: links,
    }
}

pub fn search_view(
    index: Option<&IndexSnapshot>,
    query: &Query,
    result: &SearchResult,
) -> SearchView {
    SearchView {
        kind: VIEW_KIND,
        repo: query.scope.repo.clone(),
        version: query.scope.version.clone(),
        query: Some(query.text.clone()),
        focus: Vec::new(),
        tier: Some(result.tier_reached.as_str().to_string()),
        total_ms: Some(result.trace.total_ms),
        degraded: result.degraded.map(|d| d.as_str().to_string()),
        items: result
            .evidence
            .iter()
            .map(|e| code_item(index, e))
            .collect(),
        documentation: result
            .documentation
            .iter()
            .map(|e| documentation_item(index, e))
            .collect(),
        gaps: result.gaps.iter().map(|g| g.describe()).collect(),
    }
}

pub fn focus_view(
    index: Option<&IndexSnapshot>,
    scope: &Scope,
    focus: &[String],
    evidence: &[Evidence],
) -> SearchView {
    let (documentation, code): (Vec<&Evidence>, Vec<&Evidence>) = evidence.iter().partition(|e| {
        matches!((index, e.unit), (Some(index), Some(id)) if index.unit(id).kind().is_documentation())
    });
    SearchView {
        kind: VIEW_KIND,
        repo: scope.repo.clone(),
        version: scope.version.clone(),
        query: None,
        focus: focus.to_vec(),
        tier: None,
        total_ms: None,
        degraded: None,
        items: code.into_iter().map(|e| code_item(index, e)).collect(),
        documentation: documentation
            .into_iter()
            .map(|e| documentation_item(index, e))
            .collect(),
        gaps: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::index::corpus::{Corpus, Unit};
    use harvest_search::index::vectors::HashingEmbedder;
    use harvest_search::retrieve::pack::evidence_for;
    use harvest_search::{EvidenceKey, Gap, Source, TierId};

    fn unit(name: &str, kind: UnitKind, file: &str, body: &str, lines: (u32, u32)) -> Unit {
        Unit {
            name: name.rsplit('.').next().unwrap_or(name).into(),
            qualified_name: name.into(),
            kind: Some(kind),
            file: file.into(),
            body: body.into(),
            start_line: lines.0,
            end_line: lines.1,
            ..Unit::default()
        }
    }

    async fn index() -> IndexSnapshot {
        let mut corpus = Corpus::new(Scope::new("amafu", "main"), "g");
        let class = corpus.push_unit(unit(
            "MaasClient",
            UnitKind::Class,
            "crates/installer/src/maas.rs",
            "pub struct MaasClient {}",
            (86, 90),
        ));
        let deploy = corpus.push_unit(unit(
            "MaasClient.deploy",
            UnitKind::Method,
            "crates/installer/src/maas.rs",
            &"let x = 1;\n".repeat(1_000),
            (178, 197),
        ));
        let find = corpus.push_unit(unit(
            "MaasClient.find",
            UnitKind::Method,
            "crates/installer/src/maas.rs",
            "fn find() {}",
            (160, 169),
        ));
        corpus.push_unit(unit(
            "Decision",
            UnitKind::Doc,
            "docs/decisions/0022.md",
            "`amafu install` uses `MaasClient`.",
            (5, 23),
        ));
        corpus.link(class, deploy, Relation::Contains);
        corpus.link(deploy, find, Relation::Calls);
        IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap()
    }

    fn result(index: &IndexSnapshot) -> SearchResult {
        let table = Evidence {
            key: EvidenceKey::Symbol {
                file: String::new(),
                name: "capability x".into(),
            },
            unit: None,
            label: "capability x".into(),
            start_line: 0,
            end_line: 0,
            text: "MaasClient (crates/installer/src/maas.rs:86): x = True\nunparsed row".into(),
            score: 1.0,
            found_by: Source::Plan,
            covers: Vec::new(),
        };
        SearchResult {
            evidence: vec![
                evidence_for(index.unit(1), "packed".into(), 1.0, Source::Lexical),
                table,
            ],
            documentation: vec![evidence_for(
                index.unit(3),
                "packed".into(),
                0.5,
                Source::Lexical,
            )],
            gaps: vec![Gap::Unanswered {
                hint: "MaasClient.state is missing".into(),
            }],
            tier_reached: TierId::Rounds,
            degraded: None,
            generation: None,
            trace: Default::default(),
            answer: None,
        }
    }

    #[tokio::test]
    async fn code_items_carry_their_owner_calls_and_capped_code() {
        let index = index().await;
        let query = Query::new("How does Amafu deploy?", Scope::new("amafu", "main"));
        let view = search_view(Some(&index), &query, &result(&index));
        assert_eq!(view.kind, VIEW_KIND);
        assert_eq!(view.tier.as_deref(), Some("tier1"));
        let deploy = &view.items[0];
        assert_eq!(deploy.kind, "method");
        assert_eq!(deploy.name, "MaasClient.deploy");
        assert_eq!((deploy.start_line, deploy.end_line), (178, 197));
        assert!(deploy.truncated && deploy.code.len() <= MAX_ITEM_CODE_CHARS);
        assert_eq!(
            deploy.owner.as_ref().map(|o| o.name.as_str()),
            Some("MaasClient")
        );
        assert_eq!(
            deploy.calls,
            vec![Location {
                name: "MaasClient.find".into(),
                file: "crates/installer/src/maas.rs".into(),
                line: 160
            }]
        );
        assert_eq!(view.items[1].kind, "table");
        assert_eq!(
            view.items[1].rows,
            vec![
                TableRow {
                    symbol: Some(Location {
                        name: "MaasClient".into(),
                        file: "crates/installer/src/maas.rs".into(),
                        line: 86
                    }),
                    detail: "x = True".into()
                },
                TableRow {
                    symbol: None,
                    detail: "unparsed row".into()
                }
            ]
        );
        assert_eq!(view.gaps, vec!["MaasClient.state is missing".to_string()]);
    }

    #[test]
    fn table_rows_split_the_symbol_location_from_the_detail() {
        let row = table_row("Base.run (src/base.py:12) extends Runner, Mixin");
        assert_eq!(
            row.symbol.map(|s| (s.name, s.file, s.line)),
            Some(("Base.run".into(), "src/base.py".into(), 12))
        );
        assert_eq!(row.detail, "extends Runner, Mixin");
        assert_eq!(table_row("Driver (a (b)) is odd").symbol, None);
    }

    #[tokio::test]
    async fn documentation_items_show_their_excerpt_and_the_code_they_point_to() {
        let index = index().await;
        let query = Query::new("q", Scope::new("amafu", "main"));
        let view = search_view(Some(&index), &query, &result(&index));
        let doc = &view.documentation[0];
        assert_eq!(doc.name, "Decision");
        assert!(doc.excerpt.contains("amafu install"));
        assert_eq!(
            doc.points_to
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["MaasClient"]
        );
    }

    #[tokio::test]
    async fn without_an_index_the_view_falls_back_to_the_evidence_text() {
        let index = index().await;
        let query = Query::new("q", Scope::new("amafu", "main"));
        let view = search_view(None, &query, &result(&index));
        assert_eq!(view.items[0].code, "packed");
        assert_eq!(view.items[0].kind, "method");
        assert!(view.items[0].calls.is_empty());
    }

    #[tokio::test]
    async fn focused_reads_split_code_from_documentation() {
        let index = index().await;
        let evidence = vec![
            evidence_for(index.unit(0), "packed".into(), 1.0, Source::Exact),
            evidence_for(index.unit(3), "packed".into(), 1.0, Source::Exact),
        ];
        let view = focus_view(
            Some(&index),
            &Scope::new("amafu", "main"),
            &["MaasClient".into()],
            &evidence,
        );
        assert_eq!(view.focus, vec!["MaasClient".to_string()]);
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.documentation.len(), 1);
        assert!(view.query.is_none());
    }
}
