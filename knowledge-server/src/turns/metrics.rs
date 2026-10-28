use std::time::Duration;

use crate::metrics::{CONTEXT, TURNS as TURN_COUNTS, TURN_DURATION, TURN_REJECTIONS, TURN_STAGE};
use crate::turns::admission::Rejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Lock,
    LoadContext,
    FirstToken,
    Persist,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Stage::Lock => "lock",
            Stage::LoadContext => "load_context",
            Stage::FirstToken => "first_token",
            Stage::Persist => "persist",
        }
    }
}

pub struct TurnMetrics;

pub static TURNS: TurnMetrics = TurnMetrics;

impl TurnMetrics {
    pub fn stage(&self, stage: Stage, elapsed: Duration) {
        TURN_STAGE
            .with(&[("stage", stage.label())])
            .observe(elapsed);
    }

    pub fn started(&self, history_messages: usize) {
        TURN_COUNTS.inc(&[("outcome", "started")]);
        CONTEXT
            .with(&[("unit", "messages")])
            .record(history_messages as u64);
    }

    pub fn context_tokens(&self, tokens: usize) {
        CONTEXT.with(&[("unit", "tokens")]).record(tokens as u64);
    }

    pub fn completed(&self, elapsed: Duration) {
        TURN_COUNTS.inc(&[("outcome", "completed")]);
        TURN_DURATION.observe(elapsed);
    }

    pub fn aborted(&self, count: usize) {
        TURN_COUNTS.add(&[("outcome", "aborted")], count as u64);
    }

    pub fn interrupted(&self) {
        TURN_COUNTS.inc(&[("outcome", "interrupted")]);
    }

    pub fn replayed(&self) {
        TURN_COUNTS.inc(&[("outcome", "replayed")]);
    }

    pub fn rejected(&self, rejection: Rejection) {
        let reason = match rejection {
            Rejection::NodeBusy => "node",
            Rejection::UserBusy => "user",
        };
        TURN_REJECTIONS.inc(&[("reason", reason)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejections_are_counted_by_reason() {
        let before = TURN_REJECTIONS.get(&[("reason", "user")]);
        TURNS.rejected(Rejection::UserBusy);
        assert_eq!(TURN_REJECTIONS.get(&[("reason", "user")]), before + 1);
    }
}
