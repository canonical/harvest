use std::collections::HashSet;

use crate::domain::{Evidence, Source, TokenBudget, UnitId};
use crate::index::corpus::{Relation, Unit, UnitKind};
use crate::index::graph::Direction;
use crate::index::IndexSnapshot;
use crate::retrieve::Candidate;

const CHARS_PER_TOKEN: usize = 4;
const MAX_UNIT_CHARS: usize = 8_000;
const MIN_USEFUL_CHARS: usize = 200;
const MIN_RELATIVE_SCORE: f32 = 0.2;
pub const MAX_ITEMS: usize = 12;
pub const MAX_BROAD_ITEMS: usize = 32;
const BROAD_UNIT_CHARS: usize = 1_500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Focused,
    Broad,
}

impl Shape {
    pub fn of(question: &crate::question::Question) -> Self {
        if question.broad {
            Self::Broad
        } else {
            Self::Focused
        }
    }

    fn items(self) -> usize {
        match self {
            Self::Focused => MAX_ITEMS,
            Self::Broad => MAX_BROAD_ITEMS,
        }
    }

    fn unit_chars(self) -> usize {
        match self {
            Self::Focused => MAX_UNIT_CHARS,
            Self::Broad => BROAD_UNIT_CHARS,
        }
    }
}
const SKELETON_MEMBERS: usize = 40;
const CALLEE_SIGNATURES: usize = 6;
const SIGNATURE_CHARS: usize = 160;

pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…", &text[..end])
}

fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    clip(line, SIGNATURE_CHARS).replace('\n', " ")
}

fn signature_of(unit: &Unit) -> String {
    if !unit.signature.is_empty() {
        one_line(&unit.signature)
    } else {
        one_line(&unit.body)
    }
}

fn header(unit: &Unit) -> String {
    format!(
        "{} — {}:{}-{}\n",
        unit.qualified_name, unit.file, unit.start_line, unit.end_line
    )
}

pub fn render(unit: &Unit, max_chars: usize) -> String {
    let mut text = header(unit);
    if !unit.body.is_empty() {
        text.push_str(&unit.body);
    } else {
        if !unit.signature.is_empty() {
            text.push_str(&unit.signature);
            text.push('\n');
        }
        if !unit.docstring.is_empty() {
            text.push_str(&unit.docstring);
        }
    }
    clip(&text, max_chars)
}

fn owner(index: &IndexSnapshot, unit: UnitId) -> Option<&Unit> {
    index
        .graph
        .neighbors(unit, Direction::In, &[Relation::Contains])
        .map(|o| index.unit(o))
        .find(|o| o.kind() == UnitKind::Class)
}

fn members(index: &IndexSnapshot, unit: UnitId) -> Vec<&Unit> {
    let mut found: Vec<&Unit> = index
        .graph
        .neighbors(unit, Direction::Out, &[Relation::Contains])
        .map(|m| index.unit(m))
        .filter(|m| m.kind().is_symbol())
        .collect();
    found.sort_by_key(|m| m.start_line);
    found
}

fn skeleton(index: &IndexSnapshot, unit: &Unit) -> String {
    let mut text = header(unit);
    text.push_str(&signature_of(unit));
    text.push('\n');
    if let Some(sentence) = unit.docstring.lines().find(|l| !l.trim().is_empty()) {
        text.push_str(&format!("  {}\n", one_line(sentence)));
    }
    let all = members(index, unit.id);
    for member in all.iter().take(SKELETON_MEMBERS) {
        text.push_str(&format!(
            "  {}: {}\n",
            member.start_line,
            signature_of(member)
        ));
    }
    if all.len() > SKELETON_MEMBERS {
        text.push_str(&format!(
            "  … {} more members\n",
            all.len() - SKELETON_MEMBERS
        ));
    }
    text
}

fn callees(index: &IndexSnapshot, unit: UnitId, packed: &HashSet<UnitId>) -> Vec<String> {
    let mut seen = HashSet::new();
    index
        .graph
        .neighbors(unit, Direction::Out, &[Relation::Calls])
        .filter(|c| *c != unit && !packed.contains(c) && seen.insert(*c))
        .map(|c| index.unit(c))
        .filter(|c| !c.is_vendored() && !c.is_generated())
        .take(CALLEE_SIGNATURES)
        .map(|c| format!("  {} ({}:{})", signature_of(c), c.file, c.start_line))
        .collect()
}

fn full(index: &IndexSnapshot, unit: &Unit, packed: &HashSet<UnitId>, max_chars: usize) -> String {
    let mut text = header(unit);
    if let Some(parent) = owner(index, unit.id) {
        text.push_str(&format!(
            "member of {} ({}:{}): {}\n",
            parent.qualified_name,
            parent.file,
            parent.start_line,
            signature_of(parent)
        ));
    }
    if unit.body.is_empty() {
        if !unit.signature.is_empty() {
            text.push_str(&unit.signature);
            text.push('\n');
        }
        text.push_str(&unit.docstring);
    } else {
        text.push_str(&unit.body);
    }
    let mut text = clip(&text, max_chars);
    if matches!(unit.kind(), UnitKind::Function | UnitKind::Method) {
        let calls = callees(index, unit.id, packed);
        if !calls.is_empty() {
            text.push_str("\ncalls:\n");
            text.push_str(&calls.join("\n"));
        }
    }
    text
}

pub fn render_whole(index: &IndexSnapshot, unit: UnitId, max_chars: usize) -> String {
    let unit = index.unit(unit);
    if unit.kind() == UnitKind::Class && unit.body.len() > max_chars {
        skeleton(index, unit)
    } else {
        full(index, unit, &HashSet::new(), max_chars)
    }
}

fn overlaps(a: &Unit, b: &Unit) -> bool {
    a.file == b.file && a.start_line <= b.end_line && b.start_line <= a.end_line && a.end_line > 0
}

pub fn evidence_for(unit: &Unit, text: String, score: f32, found_by: Source) -> Evidence {
    Evidence {
        key: unit.key(),
        unit: Some(unit.id),
        label: unit.label(),
        start_line: unit.start_line,
        end_line: unit.end_line,
        text,
        score,
        found_by,
        covers: Vec::new(),
    }
}

struct Packed<'a> {
    unit: &'a Unit,
    evidence: Evidence,
    whole: bool,
    rank: usize,
}

pub fn pack(
    index: &IndexSnapshot,
    candidates: &[Candidate],
    fixed: Vec<Evidence>,
    budget: TokenBudget,
) -> Vec<Evidence> {
    pack_shaped(index, candidates, fixed, budget, Shape::Focused)
}

pub fn pack_shaped(
    index: &IndexSnapshot,
    candidates: &[Candidate],
    fixed: Vec<Evidence>,
    budget: TokenBudget,
    shape: Shape,
) -> Vec<Evidence> {
    let max_unit = shape.unit_chars();
    let mut remaining = budget.0 * CHARS_PER_TOKEN;
    for evidence in &fixed {
        remaining = remaining.saturating_sub(evidence.text.len());
    }
    let top = candidates
        .iter()
        .filter(|c| !c.pinned)
        .map(|c| c.score)
        .fold(f32::MIN, f32::max);
    let mut chosen: Vec<Packed> = Vec::new();
    let mut packed_units: HashSet<UnitId> = HashSet::new();
    for (rank, candidate) in candidates.iter().enumerate() {
        if chosen.len() >= shape.items() || (remaining < MIN_USEFUL_CHARS && !chosen.is_empty()) {
            break;
        }
        if !candidate.pinned && top > 0.0 && candidate.score < top * MIN_RELATIVE_SCORE {
            continue;
        }
        let unit = index.unit(candidate.unit);
        if packed_units.contains(&unit.id) {
            continue;
        }
        let inside_whole = chosen
            .iter()
            .any(|p| p.whole && (p.unit.contains(unit) || p.unit.id == unit.id));
        if inside_whole {
            continue;
        }
        let partial_overlap = chosen
            .iter()
            .any(|p| overlaps(p.unit, unit) && !p.unit.contains(unit) && !unit.contains(p.unit));
        if partial_overlap {
            continue;
        }
        let holds_chosen = chosen.iter().any(|p| unit.contains(p.unit));
        let body_len = unit.body.len();
        let as_skeleton =
            unit.kind() == UnitKind::Class && (holds_chosen || body_len > max_unit.min(remaining));
        let text = if as_skeleton {
            skeleton(index, unit)
        } else {
            full(
                index,
                unit,
                &packed_units,
                max_unit.min(remaining.max(MIN_USEFUL_CHARS)),
            )
        };
        if text.len() > remaining && !chosen.is_empty() && !candidate.pinned {
            continue;
        }
        remaining = remaining.saturating_sub(text.len());
        let found_by = candidate
            .sources
            .first()
            .copied()
            .unwrap_or(Source::Lexical);
        packed_units.insert(unit.id);
        chosen.push(Packed {
            unit,
            evidence: evidence_for(unit, text, candidate.score, found_by),
            whole: !as_skeleton,
            rank,
        });
    }
    let mut file_rank: Vec<(String, usize)> = Vec::new();
    for packed in &chosen {
        if !file_rank.iter().any(|(f, _)| *f == packed.unit.file) {
            file_rank.push((packed.unit.file.clone(), packed.rank));
        }
    }
    chosen.sort_by_key(|p| {
        let first = file_rank
            .iter()
            .position(|(f, _)| *f == p.unit.file)
            .unwrap_or(usize::MAX);
        (first, p.unit.start_line, p.rank)
    });
    let mut out = fixed;
    out.extend(chosen.into_iter().map(|p| p.evidence));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::Corpus;
    use crate::index::vectors::HashingEmbedder;

    fn unit(name: &str, file: &str, lines: (u32, u32), body_len: usize, kind: UnitKind) -> Unit {
        Unit {
            name: name.into(),
            qualified_name: name.into(),
            file: file.into(),
            start_line: lines.0,
            end_line: lines.1,
            body: "x".repeat(body_len),
            signature: format!("def {name}():"),
            kind: Some(kind),
            ..Unit::default()
        }
    }

    fn candidates(units: &[u32]) -> Vec<Candidate> {
        units
            .iter()
            .map(|unit| Candidate {
                unit: *unit,
                score: 1.0,
                ..Candidate::default()
            })
            .collect()
    }

    #[tokio::test]
    async fn methods_stay_whole_and_their_class_becomes_a_skeleton() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        let class = corpus.push_unit(unit("Store", "a.py", (1, 400), 30_000, UnitKind::Class));
        let method = corpus.push_unit(Unit {
            parent: Some("Store".into()),
            qualified_name: "Store.save".into(),
            ..unit("save", "a.py", (100, 120), 300, UnitKind::Method)
        });
        let helper = corpus.push_unit(unit("helper", "b.py", (1, 5), 50, UnitKind::Function));
        corpus.link(class, method, Relation::Contains);
        corpus.link(method, helper, Relation::Calls);
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let packed = pack(
            &index,
            &candidates(&[method, class]),
            Vec::new(),
            TokenBudget(8_000),
        );
        assert_eq!(packed.len(), 2);
        let save = packed.iter().find(|e| e.unit == Some(method)).unwrap();
        assert!(save.text.contains("member of Store"));
        assert!(
            save.text.contains("calls:\n  def helper():"),
            "{}",
            save.text
        );
        let store = packed.iter().find(|e| e.unit == Some(class)).unwrap();
        assert!(store.text.contains("100: def save():"), "{}", store.text);
        assert!(store.text.len() < 1_000, "skeleton, not the clipped body");
    }

    #[tokio::test]
    async fn at_most_twelve_items_grouped_by_file() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        for i in 0..20u32 {
            corpus.push_unit(unit(
                &format!("f{i}"),
                &format!("{}.py", i % 3),
                (i * 10 + 1, i * 10 + 5),
                40,
                UnitKind::Function,
            ));
        }
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let ids: Vec<u32> = (0..20).collect();
        let packed = pack(&index, &candidates(&ids), Vec::new(), TokenBudget(8_000));
        assert_eq!(packed.len(), MAX_ITEMS);
        let files: Vec<&str> = packed.iter().map(|e| e.key.file()).collect();
        let mut grouped = files.clone();
        grouped.dedup();
        assert_eq!(grouped, vec!["0.py", "1.py", "2.py"]);
    }

    #[tokio::test]
    async fn weak_candidates_are_left_out_but_pinned_ones_stay() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        for (i, name) in ["strong", "weak", "pinned"].iter().enumerate() {
            corpus.push_unit(unit(
                name,
                &format!("{i}.py"),
                (1, 2),
                10,
                UnitKind::Function,
            ));
        }
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let list = vec![
            Candidate {
                unit: 2,
                pinned: true,
                ..Candidate::default()
            },
            Candidate {
                unit: 0,
                score: 5.0,
                ..Candidate::default()
            },
            Candidate {
                unit: 1,
                score: 0.5,
                ..Candidate::default()
            },
        ];
        let packed = pack(&index, &list, Vec::new(), TokenBudget(1_000));
        let mut units: Vec<u32> = packed.iter().filter_map(|e| e.unit).collect();
        units.sort();
        assert_eq!(units, vec![0, 2]);
    }

    #[test]
    fn clipping_respects_character_boundaries() {
        assert_eq!(clip("héllo", 2), "h\n…");
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[tokio::test]
    async fn broad_questions_get_more_shorter_items() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        for i in 0..40u32 {
            corpus.push_unit(unit(
                &format!("caller{i}"),
                &format!("{i}.py"),
                (1, 50),
                3_000,
                UnitKind::Function,
            ));
        }
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let ids: Vec<u32> = (0..40).collect();
        let broad = pack_shaped(
            &index,
            &candidates(&ids),
            Vec::new(),
            TokenBudget(16_000),
            Shape::Broad,
        );
        assert!(broad.len() > MAX_ITEMS, "{}", broad.len());
        assert!(broad.iter().all(|e| e.text.len() <= BROAD_UNIT_CHARS + 100));
        let question = crate::question::Question::analyze("What calls attach_volume?");
        assert_eq!(Shape::of(&question), Shape::Broad);
        let question = crate::question::Question::analyze("How does attach_volume work?");
        assert_eq!(Shape::of(&question), Shape::Focused);
    }
}
