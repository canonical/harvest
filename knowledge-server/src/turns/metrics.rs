use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::turns::admission::Rejection;

pub struct Histogram {
    count: AtomicU64,
    sum_ms: AtomicU64,
}

impl Histogram {
    const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            sum_ms: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, elapsed: Duration) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_ms
            .fetch_add(elapsed.as_millis() as u64, Ordering::Relaxed);
    }

    fn render(&self, out: &mut String, name: &str) {
        let _ = writeln!(out, "# TYPE {name}_milliseconds summary");
        let _ = writeln!(
            out,
            "{name}_milliseconds_sum {}",
            self.sum_ms.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "{name}_milliseconds_count {}",
            self.count.load(Ordering::Relaxed)
        );
    }
}

pub struct TurnMetrics {
    started: AtomicU64,
    completed: AtomicU64,
    aborted: AtomicU64,
    rejected_node: AtomicU64,
    rejected_user: AtomicU64,
    replayed: AtomicU64,
    history_messages: AtomicU64,
    pub lock: Histogram,
    pub duration: Histogram,
}

pub static TURNS: TurnMetrics = TurnMetrics {
    started: AtomicU64::new(0),
    completed: AtomicU64::new(0),
    aborted: AtomicU64::new(0),
    rejected_node: AtomicU64::new(0),
    rejected_user: AtomicU64::new(0),
    replayed: AtomicU64::new(0),
    history_messages: AtomicU64::new(0),
    lock: Histogram::new(),
    duration: Histogram::new(),
};

impl TurnMetrics {
    pub fn started(&self, history_messages: usize) {
        self.started.fetch_add(1, Ordering::Relaxed);
        self.history_messages
            .fetch_add(history_messages as u64, Ordering::Relaxed);
    }

    pub fn completed(&self, elapsed: Duration) {
        self.completed.fetch_add(1, Ordering::Relaxed);
        self.duration.observe(elapsed);
    }

    pub fn aborted(&self, count: usize) {
        self.aborted.fetch_add(count as u64, Ordering::Relaxed);
    }

    pub fn rejected(&self, rejection: Rejection) {
        match rejection {
            Rejection::NodeBusy => &self.rejected_node,
            Rejection::UserBusy => &self.rejected_user,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    pub fn replayed(&self) {
        self.replayed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self, out: &mut String) {
        let counter = |out: &mut String, name: &str, value: &AtomicU64| {
            let _ = writeln!(out, "# TYPE {name} counter");
            let _ = writeln!(out, "{name} {}", value.load(Ordering::Relaxed));
        };
        counter(out, "harvest_turns_started_total", &self.started);
        counter(out, "harvest_turns_completed_total", &self.completed);
        counter(out, "harvest_turns_aborted_total", &self.aborted);
        counter(out, "harvest_turns_replayed_total", &self.replayed);
        counter(
            out,
            "harvest_turn_history_messages_total",
            &self.history_messages,
        );
        let _ = writeln!(out, "# TYPE harvest_turns_rejected_total counter");
        let _ = writeln!(
            out,
            "harvest_turns_rejected_total{{reason=\"node\"}} {}",
            self.rejected_node.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "harvest_turns_rejected_total{{reason=\"user\"}} {}",
            self.rejected_user.load(Ordering::Relaxed)
        );
        self.lock.render(out, "harvest_turn_lock");
        self.duration.render(out, "harvest_turn_duration");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_render_in_prometheus_text_format() {
        let metrics = TurnMetrics {
            started: AtomicU64::new(2),
            completed: AtomicU64::new(1),
            aborted: AtomicU64::new(0),
            rejected_node: AtomicU64::new(3),
            rejected_user: AtomicU64::new(0),
            replayed: AtomicU64::new(0),
            history_messages: AtomicU64::new(10),
            lock: Histogram::new(),
            duration: Histogram::new(),
        };
        metrics.duration.observe(Duration::from_millis(250));
        let mut out = String::new();
        metrics.render(&mut out);
        assert!(out.contains("harvest_turns_started_total 2"));
        assert!(out.contains("harvest_turns_rejected_total{reason=\"node\"} 3"));
        assert!(out.contains("harvest_turn_duration_milliseconds_sum 250"));
        assert!(out.contains("harvest_turn_duration_milliseconds_count 1"));
    }
}
