use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::turns::metrics::TURNS;

const RETRY_AFTER_SECS: &str = "2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    NodeBusy,
    UserBusy,
}

impl Rejection {
    pub fn as_api_error(self) -> (StatusCode, Json<serde_json::Value>) {
        let (status, message) = self.describe();
        (status, Json(json!({ "error": message })))
    }

    fn describe(self) -> (StatusCode, &'static str) {
        match self {
            Rejection::NodeBusy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "this server is running as many turns as it can, retry shortly",
            ),
            Rejection::UserBusy => (
                StatusCode::TOO_MANY_REQUESTS,
                "you already have as many turns running as allowed, wait for one to finish",
            ),
        }
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        let mut response = self.as_api_error().into_response();
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static(RETRY_AFTER_SECS));
        response
    }
}

#[derive(Default)]
struct Counts {
    total: usize,
    per_user: HashMap<String, usize>,
}

pub struct TurnAdmission {
    node_limit: usize,
    user_limit: usize,
    counts: Mutex<Counts>,
}

pub struct TurnPermit {
    admission: Arc<TurnAdmission>,
    user: String,
}

impl Drop for TurnPermit {
    fn drop(&mut self) {
        self.admission.release(&self.user);
    }
}

impl TurnAdmission {
    pub fn new(node_limit: usize, user_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            node_limit,
            user_limit,
            counts: Mutex::new(Counts::default()),
        })
    }

    pub fn unlimited() -> Arc<Self> {
        Self::new(0, 0)
    }

    pub fn admit(self: &Arc<Self>, user: &str) -> Result<TurnPermit, Rejection> {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if self.node_limit > 0 && counts.total >= self.node_limit {
            TURNS.rejected(Rejection::NodeBusy);
            return Err(Rejection::NodeBusy);
        }
        let mine = counts.per_user.get(user).copied().unwrap_or(0);
        if self.user_limit > 0 && mine >= self.user_limit {
            TURNS.rejected(Rejection::UserBusy);
            return Err(Rejection::UserBusy);
        }
        counts.total += 1;
        counts.per_user.insert(user.to_string(), mine + 1);
        Ok(TurnPermit {
            admission: Arc::clone(self),
            user: user.to_string(),
        })
    }

    pub fn active(&self) -> usize {
        self.counts.lock().unwrap_or_else(|e| e.into_inner()).total
    }

    fn release(&self, user: &str) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.total = counts.total.saturating_sub(1);
        if let Some(mine) = counts.per_user.get_mut(user) {
            *mine -= 1;
            if *mine == 0 {
                counts.per_user.remove(user);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_cannot_exceed_their_share() {
        let admission = TurnAdmission::new(10, 2);
        let _a = admission.admit("alice").unwrap();
        let _b = admission.admit("alice").unwrap();
        assert_eq!(admission.admit("alice").err(), Some(Rejection::UserBusy));
        assert!(admission.admit("bob").is_ok());
    }

    #[test]
    fn the_node_limit_applies_across_users() {
        let admission = TurnAdmission::new(2, 0);
        let _a = admission.admit("a").unwrap();
        let _b = admission.admit("b").unwrap();
        assert_eq!(admission.admit("c").err(), Some(Rejection::NodeBusy));
    }

    #[test]
    fn dropping_a_permit_frees_its_slot() {
        let admission = TurnAdmission::new(1, 1);
        let permit = admission.admit("a").unwrap();
        assert_eq!(admission.active(), 1);
        drop(permit);
        assert_eq!(admission.active(), 0);
        assert!(admission.admit("a").is_ok());
    }

    #[test]
    fn zero_means_unlimited() {
        let admission = TurnAdmission::unlimited();
        let permits: Vec<_> = (0..100).map(|_| admission.admit("a").unwrap()).collect();
        assert_eq!(admission.active(), permits.len());
    }

    #[test]
    fn rejections_carry_retry_after() {
        let response = Rejection::NodeBusy.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "2");
        assert_eq!(
            Rejection::UserBusy.into_response().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
