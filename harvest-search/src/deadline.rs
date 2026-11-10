use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Deadline {
    started: Instant,
    limit: Duration,
}

impl Deadline {
    pub fn after(limit: Duration) -> Self {
        Self {
            started: Instant::now(),
            limit,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn remaining(&self) -> Duration {
        self.limit.saturating_sub(self.elapsed())
    }

    pub fn expired(&self) -> bool {
        self.remaining().is_zero()
    }

    pub fn leaves_room_for(&self, cost: Duration) -> bool {
        self.remaining() > cost
    }

    pub fn limit(&self) -> Duration {
        self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_shrinks_as_time_passes() {
        let deadline = Deadline::after(Duration::from_millis(50));
        assert!(deadline.leaves_room_for(Duration::from_millis(10)));
        assert!(!deadline.leaves_room_for(Duration::from_millis(60)));
        std::thread::sleep(Duration::from_millis(60));
        assert!(deadline.expired());
        assert!(!deadline.leaves_room_for(Duration::ZERO));
    }
}
