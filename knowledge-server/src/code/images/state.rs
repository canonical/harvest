use std::time::Duration;

use serde_json::{json, Value};

use super::failure::BuildFailure;

pub const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const STALE_BUILD: Duration = Duration::from_secs(SETUP_TIMEOUT.as_secs() + 15 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyImage {
    pub alias: String,
    pub base_commit: Option<String>,
    pub setup_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attempt {
    Building { age: Duration },
    Failed { failure: BuildFailure, setup_hash: String, age: Duration },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Image {
    pub ready: Option<ReadyImage>,
    pub attempt: Option<Attempt>,
    pub log: String,
    pub attempt_log: String,
    pub setup_warning: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageNeed {
    Keep,
    Build,
}

pub fn need(image: Option<&Image>, setup_hash: &str) -> ImageNeed {
    image.map_or(ImageNeed::Build, |image| image.need(setup_hash))
}

impl Image {
    pub fn is_building(&self) -> bool {
        matches!(self.attempt, Some(Attempt::Building { age }) if age < STALE_BUILD)
    }

    pub fn need(&self, setup_hash: &str) -> ImageNeed {
        match &self.attempt {
            Some(Attempt::Building { age }) if *age < STALE_BUILD => return ImageNeed::Keep,
            Some(Attempt::Failed { failure, setup_hash: tried, age }) if tried == setup_hash => {
                return if *age >= failure.retry_after() { ImageNeed::Build } else { ImageNeed::Keep };
            }
            _ => {}
        }
        match &self.ready {
            Some(ready) if ready.setup_hash == setup_hash => ImageNeed::Keep,
            _ => ImageNeed::Build,
        }
    }

    pub fn headline(&self) -> &'static str {
        match (&self.ready, &self.attempt) {
            (Some(_), _) => "ready",
            (None, Some(Attempt::Building { .. })) if self.is_building() => "building",
            (None, Some(Attempt::Failed { .. })) => "failed",
            _ => "none",
        }
    }

    pub fn summary(&self) -> Value {
        let attempt = self.attempt.as_ref().map(|attempt| match attempt {
            Attempt::Building { .. } => json!({ "status": "building" }),
            Attempt::Failed { failure, .. } => {
                json!({ "status": "failed", "failure": failure.as_str(), "log": self.attempt_log })
            }
        });
        json!({
            "status": self.headline(),
            "base_commit": self.ready.as_ref().and_then(|ready| ready.base_commit.clone()),
            "log": self.log,
            "setup_warning": self.setup_warning,
            "attempt": attempt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn ready(hash: &str) -> Option<ReadyImage> {
        Some(ReadyImage { alias: "a".into(), base_commit: None, setup_hash: hash.into() })
    }

    fn image(ready: Option<ReadyImage>, attempt: Option<Attempt>) -> Image {
        Image { ready, attempt, ..Image::default() }
    }

    fn failed(failure: BuildFailure, hash: &str, age: Duration) -> Option<Attempt> {
        Some(Attempt::Failed { failure, setup_hash: hash.into(), age })
    }

    #[test]
    fn a_missing_image_or_a_changed_setup_is_built() {
        assert_eq!(need(None, "h"), ImageNeed::Build);
        assert_eq!(image(ready("h"), None).need("h"), ImageNeed::Keep);
        assert_eq!(image(ready("old"), None).need("h"), ImageNeed::Build);
    }

    #[test]
    fn a_build_in_progress_is_left_alone_until_it_goes_stale() {
        assert_eq!(image(None, Some(Attempt::Building { age: MINUTE })).need("h"), ImageNeed::Keep);
        assert_eq!(image(None, Some(Attempt::Building { age: STALE_BUILD })).need("h"), ImageNeed::Build);
    }

    #[test]
    fn a_failed_build_is_retried_once_its_failure_allows() {
        let soon = image(None, failed(BuildFailure::LxdUnavailable, "h", 2 * MINUTE));
        assert_eq!(soon.need("h"), ImageNeed::Build);
        let later = image(None, failed(BuildFailure::Infrastructure, "h", 2 * MINUTE));
        assert_eq!(later.need("h"), ImageNeed::Keep);
        assert_eq!(later.need("changed"), ImageNeed::Build);
    }

    #[test]
    fn a_failed_rebuild_keeps_the_ready_image_in_use() {
        let rebuilt = image(ready("old"), failed(BuildFailure::Infrastructure, "new", MINUTE));
        assert_eq!(rebuilt.need("new"), ImageNeed::Keep);
        assert_eq!(rebuilt.headline(), "ready");
        assert_eq!(rebuilt.summary()["attempt"]["status"], "failed");
    }

    #[test]
    fn the_headline_reflects_what_sessions_can_use() {
        assert_eq!(image(None, Some(Attempt::Building { age: MINUTE })).headline(), "building");
        assert_eq!(image(None, failed(BuildFailure::Infrastructure, "h", MINUTE)).headline(), "failed");
        assert_eq!(image(None, None).headline(), "none");
    }
}
