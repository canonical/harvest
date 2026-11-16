use std::time::Duration;

use anyhow::{bail, Result};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const KEY_LABEL: &str = "harvest-preview-pass";
const TICKET_LIFETIME: Duration = Duration::from_secs(60);
pub const PASS_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Purpose {
    Ticket,
    Pass,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    pid: String,
    viewer: String,
    purpose: Purpose,
    exp: i64,
}

pub struct Passes {
    key: Vec<u8>,
}

impl Passes {
    pub fn new(server_secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEY_LABEL.as_bytes());
        hasher.update(server_secret.as_bytes());
        Self { key: hasher.finalize().to_vec() }
    }

    pub fn ticket(&self, preview_id: &str, viewer_key: &str) -> Result<String> {
        self.issue(preview_id, viewer_key, Purpose::Ticket, TICKET_LIFETIME)
    }

    pub fn pass(&self, preview_id: &str, viewer_key: &str) -> Result<String> {
        self.issue(preview_id, viewer_key, Purpose::Pass, PASS_LIFETIME)
    }

    pub fn redeem_ticket(&self, token: &str, preview_id: &str) -> Result<String> {
        self.verify(token, preview_id, Purpose::Ticket)
    }

    pub fn viewer_of_pass(&self, token: &str, preview_id: &str) -> Result<String> {
        self.verify(token, preview_id, Purpose::Pass)
    }

    fn issue(&self, preview_id: &str, viewer_key: &str, purpose: Purpose, lifetime: Duration) -> Result<String> {
        let claims = Claims {
            pid: preview_id.to_string(),
            viewer: viewer_key.to_string(),
            purpose,
            exp: Utc::now().timestamp() + lifetime.as_secs() as i64,
        };
        Ok(encode(&Header::default(), &claims, &EncodingKey::from_secret(&self.key))?)
    }

    fn verify(&self, token: &str, preview_id: &str, purpose: Purpose) -> Result<String> {
        let mut validation = Validation::default();
        validation.leeway = 0;
        let claims = decode::<Claims>(token, &DecodingKey::from_secret(&self.key), &validation)?.claims;
        if claims.purpose != purpose || claims.pid != preview_id {
            bail!("this pass is not for this preview");
        }
        Ok(claims.viewer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_names_its_viewer_for_its_preview_only() {
        let passes = Passes::new("secret");
        let ticket = passes.ticket("p1", "user:u1").unwrap();
        assert_eq!(passes.redeem_ticket(&ticket, "p1").unwrap(), "user:u1");
        assert!(passes.redeem_ticket(&ticket, "p2").is_err());
    }

    #[test]
    fn tickets_and_passes_cannot_stand_in_for_each_other() {
        let passes = Passes::new("secret");
        let ticket = passes.ticket("p1", "anon:a").unwrap();
        let pass = passes.pass("p1", "anon:a").unwrap();
        assert!(passes.viewer_of_pass(&ticket, "p1").is_err());
        assert!(passes.redeem_ticket(&pass, "p1").is_err());
        assert_eq!(passes.viewer_of_pass(&pass, "p1").unwrap(), "anon:a");
    }

    #[test]
    fn passes_from_another_key_or_tampered_ones_are_rejected() {
        let pass = Passes::new("secret").pass("p1", "user:u1").unwrap();
        assert!(Passes::new("other").viewer_of_pass(&pass, "p1").is_err());
        let mut tampered = pass.clone();
        tampered.pop();
        assert!(Passes::new("secret").viewer_of_pass(&tampered, "p1").is_err());
    }

    #[test]
    fn the_preview_key_differs_from_the_login_key() {
        let pass = Passes::new("secret").pass("p1", "user:u1").unwrap();
        assert!(crate::auth::jwt::validate("secret", &pass).is_err());
    }
}
