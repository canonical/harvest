use crate::domain::{Evidence, Source, TokenBudget};
use crate::index::corpus::Unit;
use crate::index::IndexSnapshot;
use crate::retrieve::Candidate;

const CHARS_PER_TOKEN: usize = 4;
const MAX_UNIT_CHARS: usize = 6_000;
const MIN_USEFUL_CHARS: usize = 200;
const MIN_RELATIVE_SCORE: f32 = 0.2;

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

pub fn render(unit: &Unit, max_chars: usize) -> String {
    let mut text = format!(
        "{} — {}:{}-{}\n",
        unit.qualified_name, unit.file, unit.start_line, unit.end_line
    );
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

pub fn pack(
    index: &IndexSnapshot,
    candidates: &[Candidate],
    fixed: Vec<Evidence>,
    budget: TokenBudget,
) -> Vec<Evidence> {
    let mut remaining = budget.0 * CHARS_PER_TOKEN;
    let mut packed: Vec<Evidence> = Vec::new();
    let mut units: Vec<&Unit> = Vec::new();
    for evidence in fixed {
        remaining = remaining.saturating_sub(evidence.text.len());
        packed.push(evidence);
    }
    let top = candidates
        .iter()
        .filter(|c| !c.pinned)
        .map(|c| c.score)
        .fold(f32::MIN, f32::max);
    for candidate in candidates {
        if remaining < MIN_USEFUL_CHARS && !packed.is_empty() {
            break;
        }
        if !candidate.pinned && top > 0.0 && candidate.score < top * MIN_RELATIVE_SCORE {
            continue;
        }
        let unit = index.unit(candidate.unit);
        if units.iter().any(|u| overlaps(u, unit)) {
            continue;
        }
        let text = render(unit, MAX_UNIT_CHARS.min(remaining.max(MIN_USEFUL_CHARS)));
        remaining = remaining.saturating_sub(text.len());
        let found_by = candidate
            .sources
            .first()
            .copied()
            .unwrap_or(Source::Lexical);
        packed.push(evidence_for(unit, text, candidate.score, found_by));
        units.push(unit);
    }
    packed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::Corpus;
    use crate::index::vectors::HashingEmbedder;

    fn unit(name: &str, file: &str, lines: (u32, u32), body_len: usize) -> Unit {
        Unit {
            name: name.into(),
            file: file.into(),
            start_line: lines.0,
            end_line: lines.1,
            body: "x".repeat(body_len),
            ..Unit::default()
        }
    }

    #[tokio::test]
    async fn packing_respects_the_budget_and_skips_overlapping_spans() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        corpus.push_unit(unit("Class", "a.py", (1, 50), 1_000));
        corpus.push_unit(unit("Class.method", "a.py", (10, 20), 300));
        corpus.push_unit(unit("other", "b.py", (1, 5), 2_000));
        corpus.push_unit(unit("third", "c.py", (1, 5), 8_000));
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let candidates: Vec<Candidate> = (0..4)
            .map(|unit| Candidate {
                unit,
                score: 1.0,
                ..Candidate::default()
            })
            .collect();
        let packed = pack(&index, &candidates, Vec::new(), TokenBudget(1_000));
        let labels: Vec<u32> = packed.iter().filter_map(|e| e.unit).collect();
        assert_eq!(labels, vec![0, 2, 3]);
        let total: usize = packed.iter().map(|e| e.text.len()).sum();
        assert!(total <= 1_000 * CHARS_PER_TOKEN + MIN_USEFUL_CHARS);
        assert!(packed[2].text.ends_with('…'));
    }

    #[tokio::test]
    async fn weak_candidates_are_left_out_but_pinned_ones_stay() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        for (i, name) in ["strong", "weak", "pinned"].iter().enumerate() {
            corpus.push_unit(unit(name, &format!("{i}.py"), (1, 2), 10));
        }
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let candidates = vec![
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
        let packed = pack(&index, &candidates, Vec::new(), TokenBudget(1_000));
        assert_eq!(
            packed.iter().filter_map(|e| e.unit).collect::<Vec<_>>(),
            vec![2, 0]
        );
    }

    #[test]
    fn clipping_respects_character_boundaries() {
        assert_eq!(clip("héllo", 2), "h\n…");
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }
}
