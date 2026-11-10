use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use crate::deadline::Deadline;
use crate::domain::{Degraded, Evidence, EvidenceKey, Gap, Query, SearchResult, TierId, UnitId};
use crate::index::IndexSnapshot;
use crate::question::Question;
use crate::retrieve::{Candidate, Terms};
use crate::trace::Trace;

const POOL: usize = 200;

pub struct SearchContext {
    pub query: Query,
    pub question: Question,
    pub terms: Terms,
    pub index: Arc<IndexSnapshot>,
    pub deadline: Deadline,
    pub candidates: Vec<Candidate>,
    pub evidence: Vec<Evidence>,
    pub fixed: Vec<Evidence>,
    pub gaps: Vec<Gap>,
    pub trace: Trace,
    pub tier: TierId,
    pub stable: bool,
    pub degraded: Option<Degraded>,
    pub notes: Vec<String>,
    pub named: Vec<UnitId>,
}

impl SearchContext {
    pub fn new(query: &Query, index: Arc<IndexSnapshot>, deadline: Deadline) -> Self {
        let question = Question::analyze(&query.text);
        let named = index.named_in(&question);
        Self {
            named,
            question,
            terms: Terms::from_question(&query.text),
            query: query.clone(),
            index,
            deadline,
            candidates: Vec::new(),
            evidence: Vec::new(),
            fixed: Vec::new(),
            gaps: Vec::new(),
            trace: Trace::default(),
            tier: TierId::Rounds,
            stable: false,
            degraded: None,
            notes: Vec::new(),
        }
    }

    pub fn evidence_keys(&self) -> HashSet<&EvidenceKey> {
        self.evidence.iter().flat_map(Evidence::keys).collect()
    }

    pub fn evidence_units(&self) -> HashSet<UnitId> {
        self.evidence.iter().filter_map(|e| e.unit).collect()
    }

    pub fn has_unit(&self, unit: UnitId) -> bool {
        let target = self.index.unit(unit);
        let key = target.key();
        self.evidence.iter().any(|e| {
            e.unit == Some(unit)
                || e.covers.contains(&key)
                || e.unit.is_some_and(|u| self.index.unit(u).contains(target))
        })
    }

    pub fn mark_truncated(&mut self) {
        if self.degraded.is_none() {
            self.degraded = Some(Degraded::DeadlineTruncated);
        }
    }

    pub fn into_result(mut self, started: Instant) -> SearchResult {
        self.trace.pool = self
            .candidates
            .iter()
            .take(POOL)
            .map(|c| self.index.unit(c.unit).key())
            .collect();
        self.trace.total_ms = started.elapsed().as_secs_f64() * 1000.0;
        SearchResult {
            evidence: self.evidence,
            gaps: self.gaps,
            tier_reached: self.tier,
            degraded: self.degraded,
            generation: Some(self.index.generation.clone()),
            trace: self.trace,
            answer: None,
        }
    }
}
