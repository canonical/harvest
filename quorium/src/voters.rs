use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use tokio::time::Instant;

use crate::types::{LogId, ServerId};

pub const MAX_VOTERS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tuning {
    pub snapshot_logs: u64,
    pub caught_up_lag: u64,
    pub control_tick: Duration,
    pub demote_after: Option<Duration>,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            snapshot_logs: 1_000,
            caught_up_lag: 16,
            control_tick: Duration::from_millis(200),
            demote_after: None,
        }
    }
}

pub fn target_voters(servers: usize) -> usize {
    let odd = if servers % 2 == 0 {
        servers.saturating_sub(1)
    } else {
        servers
    };
    odd.clamp(1, MAX_VOTERS)
}

pub type Replication = BTreeMap<ServerId, Option<LogId>>;

fn caught_up(replication: &Replication, id: ServerId, commit: u64, lag: u64) -> bool {
    replication
        .get(&id)
        .and_then(|matched| *matched)
        .is_some_and(|matched| matched.index + lag >= commit)
}

pub fn promotion_candidate(
    voters: &BTreeSet<ServerId>,
    learners: &[ServerId],
    replication: &Replication,
    commit: u64,
    lag: u64,
) -> Option<ServerId> {
    if voters.len() >= target_voters(voters.len() + learners.len()) {
        return None;
    }
    learners
        .iter()
        .copied()
        .find(|id| caught_up(replication, *id, commit, lag))
}

#[derive(Default)]
pub struct VoterWatch {
    absent_since: HashMap<ServerId, Instant>,
}

impl VoterWatch {
    pub fn observe(&mut self, absent: &BTreeSet<ServerId>, now: Instant) {
        self.absent_since.retain(|id, _| absent.contains(id));
        for id in absent {
            self.absent_since.entry(*id).or_insert(now);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn demotion(
        &self,
        leader: ServerId,
        voters: &BTreeSet<ServerId>,
        learners: &[ServerId],
        replication: &Replication,
        commit: u64,
        tuning: &Tuning,
        now: Instant,
    ) -> Option<(ServerId, ServerId)> {
        let after = tuning.demote_after?;
        let replacement = learners
            .iter()
            .copied()
            .find(|id| caught_up(replication, *id, commit, tuning.caught_up_lag))?;
        voters
            .iter()
            .copied()
            .filter(|id| *id != leader)
            .find(|id| {
                self.absent_since
                    .get(id)
                    .is_some_and(|since| now.duration_since(*since) >= after)
            })
            .map(|stale| (stale, replacement))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openraft::CommittedLeaderId;

    fn at(index: u64) -> Option<LogId> {
        Some(LogId::new(CommittedLeaderId::new(1, 0), index))
    }

    fn tuning(demote_after: Option<Duration>) -> Tuning {
        Tuning {
            demote_after,
            ..Tuning::default()
        }
    }

    #[test]
    fn voter_targets_stay_odd_and_bounded() {
        assert_eq!(target_voters(0), 1);
        assert_eq!(target_voters(2), 1);
        assert_eq!(target_voters(3), 3);
        assert_eq!(target_voters(4), 3);
        assert_eq!(target_voters(9), 5);
    }

    #[test]
    fn only_caught_up_learners_are_promoted_and_only_when_voters_are_missing() {
        let voters = BTreeSet::from([1]);
        let replication = Replication::from([(2, at(5)), (3, at(100))]);
        assert_eq!(
            promotion_candidate(&voters, &[2, 3], &replication, 100, 16),
            Some(3)
        );
        let full = BTreeSet::from([1, 2, 3]);
        assert_eq!(
            promotion_candidate(&full, &[4], &replication, 100, 16),
            None
        );
    }

    #[test]
    fn demotion_is_off_unless_configured() {
        let mut watch = VoterWatch::default();
        let start = Instant::now();
        watch.observe(&BTreeSet::from([2]), start);
        let replication = Replication::from([(4, at(100))]);
        let voters = BTreeSet::from([1, 2, 3]);
        let later = start + Duration::from_secs(3_600);
        assert_eq!(
            watch.demotion(1, &voters, &[4], &replication, 100, &tuning(None), later),
            None
        );
    }

    #[test]
    fn a_voter_absent_for_long_enough_is_replaced_by_a_caught_up_learner() {
        let start = Instant::now();
        let mut watch = VoterWatch::default();
        let voters = BTreeSet::from([1, 2, 3]);
        let replication = Replication::from([(4, at(100))]);
        let policy = tuning(Some(Duration::from_secs(60)));
        watch.observe(&BTreeSet::from([2]), start);
        let soon = start + Duration::from_secs(10);
        assert_eq!(
            watch.demotion(1, &voters, &[4], &replication, 100, &policy, soon),
            None
        );
        let later = start + Duration::from_secs(61);
        assert_eq!(
            watch.demotion(1, &voters, &[4], &replication, 100, &policy, later),
            Some((2, 4))
        );
    }

    #[test]
    fn coming_back_resets_the_clock_and_no_replacement_means_no_demotion() {
        let start = Instant::now();
        let mut watch = VoterWatch::default();
        let voters = BTreeSet::from([1, 2, 3]);
        let policy = tuning(Some(Duration::from_secs(60)));
        watch.observe(&BTreeSet::from([2]), start);
        watch.observe(&BTreeSet::new(), start + Duration::from_secs(30));
        watch.observe(&BTreeSet::from([2]), start + Duration::from_secs(40));
        let replication = Replication::from([(4, at(100))]);
        let at_70 = start + Duration::from_secs(70);
        assert_eq!(
            watch.demotion(1, &voters, &[4], &replication, 100, &policy, at_70),
            None
        );
        let much_later = start + Duration::from_secs(500);
        assert_eq!(
            watch.demotion(1, &voters, &[], &replication, 100, &policy, much_later),
            None
        );
        assert_eq!(
            watch.demotion(2, &voters, &[4], &replication, 100, &policy, much_later),
            None
        );
    }
}
