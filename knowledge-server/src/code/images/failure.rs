use std::time::Duration;

const LXD_RETRY: Duration = Duration::from_secs(60);
const INFRASTRUCTURE_RETRY: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildFailure {
    LxdUnavailable,
    Infrastructure,
}

impl BuildFailure {
    pub fn of(error: &anyhow::Error) -> Self {
        let lxd_unreachable = error.chain().any(|cause| {
            cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|request| request.is_connect() || request.is_timeout())
        });
        if lxd_unreachable {
            Self::LxdUnavailable
        } else {
            Self::Infrastructure
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lxd_unavailable" => Some(Self::LxdUnavailable),
            "infrastructure" => Some(Self::Infrastructure),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LxdUnavailable => "lxd_unavailable",
            Self::Infrastructure => "infrastructure",
        }
    }

    pub fn retry_after(self) -> Duration {
        match self {
            Self::LxdUnavailable => LXD_RETRY,
            Self::Infrastructure => INFRASTRUCTURE_RETRY,
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;

    #[tokio::test]
    async fn a_refused_connection_to_lxd_is_worth_retrying_soon() {
        let refused = reqwest::Client::new().get("http://127.0.0.1:9").send().await.unwrap_err();
        let error = Err::<(), _>(refused).context("creating LXD container").unwrap_err();
        assert_eq!(BuildFailure::of(&error), BuildFailure::LxdUnavailable);
        assert!(BuildFailure::LxdUnavailable.retry_after() < BuildFailure::Infrastructure.retry_after());
    }

    #[test]
    fn other_errors_are_infrastructure_failures() {
        let error = anyhow::anyhow!("publishing LXD image").context("building");
        assert_eq!(BuildFailure::of(&error), BuildFailure::Infrastructure);
    }

    #[test]
    fn failures_round_trip_through_their_stored_names() {
        for failure in [BuildFailure::LxdUnavailable, BuildFailure::Infrastructure] {
            assert_eq!(BuildFailure::parse(failure.as_str()), Some(failure));
        }
        assert_eq!(BuildFailure::parse("setup"), None);
    }
}
