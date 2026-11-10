use harvest_search::{Evidence, EvidenceKey};

use crate::gold::{GoldEvidence, GoldQuery};

const MIN_SPAN_OVERLAP: f64 = 0.5;

fn normalize(path: &str) -> &str {
    path.trim_start_matches("./")
}

fn same_name(gold: &str, found: &str) -> bool {
    gold == found
        || found.ends_with(&format!(".{gold}"))
        || gold.ends_with(&format!(".{found}"))
        || found.ends_with(&format!("::{gold}"))
}

fn span_overlap(gold: (u32, u32), found: (u32, u32)) -> f64 {
    let (gs, ge) = (gold.0, gold.1.max(gold.0));
    let (fs, fe) = (found.0, found.1.max(found.0));
    if fe == 0 || fs > ge || gs > fe {
        return 0.0;
    }
    let overlap = ge.min(fe) - gs.max(fs) + 1;
    f64::from(overlap) / f64::from(ge - gs + 1)
}

pub fn matches_key(gold: &GoldEvidence, key: &EvidenceKey, lines: Option<(u32, u32)>) -> bool {
    if normalize(key.file()) != normalize(&gold.file) {
        return false;
    }
    let gold_lines = gold.start_line.map(|s| (s, gold.end_line.unwrap_or(s)));
    if let (Some(name), EvidenceKey::Symbol { name: found, .. }) = (&gold.name, key) {
        if same_name(name, found) {
            return true;
        }
    }
    match (gold_lines, lines) {
        (Some(g), Some(f)) => span_overlap(g, f) >= MIN_SPAN_OVERLAP,
        (None, _) if gold.name.is_none() => true,
        _ => false,
    }
}

fn evidence_lines(evidence: &Evidence, key: &EvidenceKey) -> Option<(u32, u32)> {
    match key {
        EvidenceKey::Span {
            start_line,
            end_line,
            ..
        } => Some((*start_line, *end_line)),
        EvidenceKey::Symbol { .. } if std::ptr::eq(key, &evidence.key) && evidence.end_line > 0 => {
            Some((evidence.start_line, evidence.end_line))
        }
        EvidenceKey::Symbol { .. } => None,
    }
}

pub fn found_in_evidence(gold: &GoldEvidence, evidence: &[Evidence]) -> bool {
    evidence.iter().any(|e| {
        e.keys()
            .any(|key| matches_key(gold, key, evidence_lines(e, key)))
    })
}

pub fn found_in_pool(gold: &GoldEvidence, pool: &[EvidenceKey]) -> bool {
    pool.iter().any(|key| {
        let lines = match key {
            EvidenceKey::Span {
                start_line,
                end_line,
                ..
            } => Some((*start_line, *end_line)),
            EvidenceKey::Symbol { .. } => None,
        };
        matches_key(gold, key, lines)
    })
}

fn ratio(found: usize, total: usize) -> f64 {
    if total == 0 {
        1.0
    } else {
        found as f64 / total as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recall {
    pub essential: f64,
    pub all: f64,
}

pub fn recall(gold: &GoldQuery, evidence: &[Evidence]) -> Recall {
    let essential: Vec<_> = gold.essential().collect();
    Recall {
        essential: ratio(
            essential
                .iter()
                .filter(|g| found_in_evidence(g, evidence))
                .count(),
            essential.len(),
        ),
        all: ratio(
            gold.evidence
                .iter()
                .filter(|g| found_in_evidence(g, evidence))
                .count(),
            gold.evidence.len(),
        ),
    }
}

pub fn pool_recall(gold: &GoldQuery, pool: &[EvidenceKey]) -> Option<f64> {
    if pool.is_empty() {
        return None;
    }
    let essential: Vec<_> = gold.essential().collect();
    Some(ratio(
        essential.iter().filter(|g| found_in_pool(g, pool)).count(),
        essential.len(),
    ))
}

pub fn ndcg(gold: &GoldQuery, evidence: &[Evidence], k: usize) -> f64 {
    let mut credited = vec![false; gold.evidence.len()];
    let mut dcg = 0.0;
    for (rank, e) in evidence.iter().take(k).enumerate() {
        let mut gain = 0.0f64;
        for (i, g) in gold.evidence.iter().enumerate() {
            if !credited[i] && found_in_evidence(g, std::slice::from_ref(e)) {
                credited[i] = true;
                gain += if g.essential { 2.0 } else { 1.0 };
            }
        }
        dcg += gain / (rank as f64 + 2.0).log2();
    }
    let mut ideal: Vec<f64> = gold
        .evidence
        .iter()
        .map(|g| if g.essential { 2.0 } else { 1.0 })
        .collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let idcg: f64 = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(rank, gain)| gain / (rank as f64 + 2.0).log2())
        .sum();
    if idcg == 0.0 {
        1.0
    } else {
        dcg / idcg
    }
}

pub fn precision(gold: &GoldQuery, evidence: &[Evidence]) -> f64 {
    let total: usize = evidence.iter().map(|e| e.text.len().max(1)).sum();
    if total == 0 {
        return 0.0;
    }
    let useful: usize = evidence
        .iter()
        .filter(|e| {
            gold.evidence
                .iter()
                .any(|g| found_in_evidence(g, std::slice::from_ref(e)))
        })
        .map(|e| e.text.len().max(1))
        .sum();
    useful as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::Source;

    fn symbol(file: &str, name: &str, lines: (u32, u32)) -> Evidence {
        Evidence {
            key: EvidenceKey::Symbol {
                file: file.into(),
                name: name.into(),
            },
            unit: Some(0),
            label: name.into(),
            start_line: lines.0,
            end_line: lines.1,
            text: "x".repeat(100),
            score: 1.0,
            found_by: Source::Lexical,
            covers: Vec::new(),
        }
    }

    fn query(evidence: Vec<GoldEvidence>) -> GoldQuery {
        GoldQuery {
            id: "q".into(),
            question: "?".into(),
            repo: "r".into(),
            version: "v".into(),
            kind: String::new(),
            source: String::new(),
            evidence,
            reference_answer: None,
        }
    }

    #[test]
    fn names_match_bare_or_qualified_and_spans_need_half_overlap() {
        let gold = GoldEvidence::symbol("a.py", "initialize_connection", (40, 60), true);
        assert!(found_in_evidence(
            &gold,
            &[symbol("a.py", "Driver.initialize_connection", (40, 60))]
        ));
        assert!(
            found_in_evidence(&gold, &[symbol("a.py", "Driver", (1, 200))]),
            "a containing class counts"
        );
        assert!(!found_in_evidence(
            &gold,
            &[symbol("a.py", "other", (55, 70))]
        ));
        assert!(!found_in_evidence(
            &gold,
            &[symbol("b.py", "initialize_connection", (40, 60))]
        ));
        let span = GoldEvidence::span("a.py", (10, 19), true);
        assert!(found_in_evidence(&span, &[symbol("a.py", "x", (15, 30))]));
        assert!(!found_in_evidence(&span, &[symbol("a.py", "x", (16, 30))]));
        assert!(found_in_evidence(
            &GoldEvidence::file("a.py", true),
            &[symbol("a.py", "x", (1, 1))]
        ));
    }

    #[test]
    fn covered_keys_count_toward_recall() {
        let mut table = symbol("", "capability x", (0, 0));
        table.covers = vec![EvidenceKey::Symbol {
            file: "drivers/lvm.py".into(),
            name: "LVMDriver".into(),
        }];
        let gold = query(vec![GoldEvidence::symbol(
            "drivers/lvm.py",
            "LVMDriver",
            (1, 90),
            true,
        )]);
        assert_eq!(recall(&gold, &[table]).essential, 1.0);
    }

    #[test]
    fn recall_ndcg_and_precision_reflect_rank_and_noise() {
        let gold = query(vec![
            GoldEvidence::symbol("a.py", "f", (1, 5), true),
            GoldEvidence::symbol("b.py", "g", (1, 5), false),
        ]);
        let perfect = vec![symbol("a.py", "f", (1, 5)), symbol("b.py", "g", (1, 5))];
        let reversed = vec![
            symbol("z.py", "noise", (1, 5)),
            symbol("b.py", "g", (1, 5)),
            symbol("a.py", "f", (1, 5)),
        ];
        assert_eq!(
            recall(&gold, &perfect),
            Recall {
                essential: 1.0,
                all: 1.0
            }
        );
        assert!((ndcg(&gold, &perfect, 10) - 1.0).abs() < 1e-9);
        assert!(ndcg(&gold, &reversed, 10) < 0.8);
        assert!((precision(&gold, &reversed) - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(recall(&gold, &[]).essential, 0.0);
        assert_eq!(
            pool_recall(
                &gold,
                &[EvidenceKey::Symbol {
                    file: "a.py".into(),
                    name: "f".into()
                }]
            ),
            Some(1.0)
        );
    }
}
