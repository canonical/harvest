use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinToken {
    #[serde(rename = "c")]
    pub cluster_id: String,
    #[serde(rename = "a")]
    pub addresses: Vec<String>,
    #[serde(rename = "s")]
    pub secret: String,
    #[serde(rename = "k", default, skip_serializing_if = "String::is_empty")]
    pub shared_secret: String,
}

impl JoinToken {
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("serializing a join token"))
    }

    pub fn decode(token: &str) -> Result<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(token.trim())
            .context("the join token is not valid base64")?;
        serde_json::from_slice(&bytes).context("the join token is malformed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        let token = JoinToken {
            cluster_id: "c".into(),
            addresses: vec!["http://10.0.0.1:8081".into()],
            secret: "s".into(),
            shared_secret: "k".into(),
        };
        assert_eq!(JoinToken::decode(&token.encode()).unwrap(), token);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(JoinToken::decode("not a token!").is_err());
    }
}
