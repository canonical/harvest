use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

use crate::neo4j::Neo4jClient;
use super::build_http_client;

const IDENTITY_ID: &str = "singleton";

/// A Harvest-generated (or previously persisted) LXD client identity.
pub struct LxdIdentity {
    pub client_cert: String,
    pub client_key: String,
    pub trusted: bool,
}

/// Loads the persisted singleton identity, generating and persisting a new
/// self-signed one (untrusted) if none exists yet.
pub async fn load_or_generate(neo4j: &Neo4jClient) -> Result<LxdIdentity> {
    let rows = neo4j.query_read(
        "MATCH (i:LxdIdentity {id: $id})
         RETURN i.client_cert AS client_cert, i.client_key AS client_key, i.trusted AS trusted",
        json!({ "id": IDENTITY_ID }),
    ).await?;

    if let Some(row) = rows.into_iter().next() {
        return Ok(LxdIdentity {
            client_cert: row["client_cert"].as_str().unwrap_or_default().to_string(),
            client_key:  row["client_key"].as_str().unwrap_or_default().to_string(),
            trusted:     row["trusted"].as_bool().unwrap_or(false),
        });
    }

    let certified_key = rcgen::generate_simple_self_signed(vec!["harvest".to_string()])
        .context("generating LXD client identity")?;
    let client_cert = certified_key.cert.pem();
    let client_key  = certified_key.signing_key.serialize_pem();
    let now = chrono::Utc::now().to_rfc3339();

    neo4j.query_read(
        "MERGE (i:LxdIdentity {id: $id})
         SET i.client_cert = $cert, i.client_key = $key, i.trusted = false, i.created_at = $now",
        json!({ "id": IDENTITY_ID, "cert": client_cert, "key": client_key, "now": now }),
    ).await?;

    Ok(LxdIdentity { client_cert, client_key, trusted: false })
}

/// Marks the persisted identity as trusted after a successful join.
pub async fn mark_trusted(neo4j: &Neo4jClient) -> Result<()> {
    neo4j.query_read(
        "MATCH (i:LxdIdentity {id: $id}) SET i.trusted = true",
        json!({ "id": IDENTITY_ID }),
    ).await?;
    Ok(())
}

/// Extracts the raw DER bytes from a PEM certificate block. LXD's
/// `POST /1.0/certificates` expects the `certificate` field as base64 of the
/// raw DER (`base64.StdEncoding.DecodeString` -> `x509.ParseCertificate` on
/// the server side) — not the PEM text itself.
fn pem_cert_to_der(pem: &str) -> Result<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END:   &str = "-----END CERTIFICATE-----";

    let start = pem.find(BEGIN).map(|i| i + BEGIN.len())
        .ok_or_else(|| anyhow::anyhow!("no PEM certificate block found"))?;
    let end = pem.find(END)
        .ok_or_else(|| anyhow::anyhow!("unterminated PEM certificate block"))?;
    if end < start {
        bail!("malformed PEM certificate block");
    }

    let body: String = pem[start..end].chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD.decode(body.as_bytes()).context("decoding PEM certificate body")
}

/// Self-registers `identity`'s certificate with an LXD server using a
/// one-time trust token (`lxc config trust add --name <name>`, no cert
/// argument). This is called over an mTLS connection using `identity`'s own
/// (not-yet-trusted) client cert — LXD accepts `POST /1.0/certificates` from
/// untrusted callers exactly when a valid `trust_token` is supplied.
pub async fn join_with_token(
    endpoint: &str,
    identity: &LxdIdentity,
    name: &str,
    token: &str,
    ca_cert: Option<&str>,
    insecure: bool,
) -> Result<()> {
    let http = build_http_client(&identity.client_cert, &identity.client_key, ca_cert, insecure)?;
    let der = pem_cert_to_der(&identity.client_cert)?;

    let url = format!("{}/1.0/certificates", endpoint.trim_end_matches('/'));
    let resp = http.post(&url)
        .json(&json!({
            "type":        "client",
            "name":        name,
            "certificate": STANDARD.encode(der),
            "trust_token": token,
        }))
        .send().await.context("submitting LXD trust-token join request")?;

    let status = resp.status();
    if !status.is_success() {
        let body: Value = resp.json().await.unwrap_or_default();
        bail!("LXD trust-token join failed ({status}): {body}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    const TEST_CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIDFzCCAf+gAwIBAgIUfa1whA4eLVBTEndTkfzURCCkn1gwDQYJKoZIhvcNAQEL\nBQAwGzEZMBcGA1UEAwwQaGFydmVzdC1seGQtdGVzdDAeFw0yNjA3MDMyMDA1MzVa\nFw0zNjA2MzAyMDA1MzVaMBsxGTAXBgNVBAMMEGhhcnZlc3QtbHhkLXRlc3QwggEi\nMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC6adiI5Nwy9MDlzR92GgFc7b2L\n/ka6ccF3I6RyUxfUveDyY7WzkFOjWS6gWrM0mg+zTkP0EyEWdLaKZsZokum1gDIl\nNtd3A96d8Kz4MYxf7s+d7P+5NOyS0XaqwZmqRxkq+Ps/xrULcuBydLnkDse43+DU\n1AoueIiHZL2lDgGU0LhTUi8COln/daye2zjGv7dzaOzJCRq4YWc+D45ha8ii3GAr\natzLR8OjS9eMFrVivT5PLvGArp7qzVGvgQZ4AhTw9DELACt6gF85y4NOZ3+QhQb6\nVKza5MkNmBC6piEmOgBxOEzVwBwTw0PoQfESgGi6i2sWf5+jysCtLZAdHsSvAgMB\nAAGjUzBRMB0GA1UdDgQWBBQKFOj0BLCTRijKht/LnCciyhht1TAfBgNVHSMEGDAW\ngBQKFOj0BLCTRijKht/LnCciyhht1TAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3\nDQEBCwUAA4IBAQBTpVu67w2AzGjw2rWk32ZQp05ldBsc1PR3l/E0gxOhip4zpc3l\nOEg9KZtwtn9zkIwbRE8xDsKMY2acy1AroqcR0IUA/XXZQOXWjqKlQJRYePcIju6v\ngy636poDLDVWT09GzFEdh7adOll24i2ghRKxX+1gP6yoK7VKbh3u1K7SAPX4Cw+V\nUsTKgdy0ott1Calzr1rgLRDYxy2sAjtT98HQs+06J+JioHfkVowVBBsf8QV9Qq/d\nZCdcQ+Ej+sDvw5h5ynKUvm+LKHk6d6aKyh8cNL67Lz14znBoX/RElz/UnhSgvlc1\nklOERsYlhYR8s0qCGY8QIJfCCdOlvsuLMS5+\n-----END CERTIFICATE-----\n";
    const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQC6adiI5Nwy9MDl\nzR92GgFc7b2L/ka6ccF3I6RyUxfUveDyY7WzkFOjWS6gWrM0mg+zTkP0EyEWdLaK\nZsZokum1gDIlNtd3A96d8Kz4MYxf7s+d7P+5NOyS0XaqwZmqRxkq+Ps/xrULcuBy\ndLnkDse43+DU1AoueIiHZL2lDgGU0LhTUi8COln/daye2zjGv7dzaOzJCRq4YWc+\nD45ha8ii3GAratzLR8OjS9eMFrVivT5PLvGArp7qzVGvgQZ4AhTw9DELACt6gF85\ny4NOZ3+QhQb6VKza5MkNmBC6piEmOgBxOEzVwBwTw0PoQfESgGi6i2sWf5+jysCt\nLZAdHsSvAgMBAAECggEAMl92zWs2m6hq1c5LoaDeXGu77C/+kdQ6iMS/Y8tTZcAX\noLhT+d1W1I29XUSVJ3I4KuZL05E1wDkyuIyUMd79O3gUVN0QdU884WYPf5P4EFZa\nkRzhb30/Ll9e1z6wlQRYZzXXwwChnKHix9sF/nwF+U26FhjkVXFpx1hwLMFvqPQV\nozsQGU2yxLpK1BU8p6peSYcbNy/klCizHdFrUQfI8YP4Vx6a1S7An1C7S857Xcfz\nH/QA8w7RZrW5zNznIyXM+gPC3nzY45FRBXKP8QVirYJi+DaJPCwfH9Kb7Vm090dr\nAMh9koJa1pY75bv7e6PDw1JRhRfbKBDnkn27gtY8qQKBgQDksHSZAvVJQMvPSXwr\nI+SUwDquE1hH72iRGpk3Fv1KuBqDQXvK+ITY6SL2T0VjRVFZEaX6iaPQJ8nyxBpc\neGYi7Y+UqlCN++O8Kze+yMTsQcbL5xzs2rZnZS3Sc4JMJMgn2f8+9GrPFTuSId0h\nxEGTBiG2HwPZ2J+JXwVglKp/aQKBgQDQrOx4aJwuOHyNjibRX1zjkv1f41yB+lrz\nBxI0phmNNr50Gk9fez0+CIokZbkGmHKj7K0ctR0siqKC7UyK/Zuj1ym0z6aRXhzm\nQ5lORHcA7ODYRSlYiEZUeamZI32gCnPB4NDlpod6Yqtr5b/iNjHy9/vrZnevck9S\n7JA41Yq4VwKBgB3+1wxKywl0qkbiCJtP9edc31V9zBKDYF/H8Vi8dzSZuUCGEkqp\nFiOtUJymAR/oM6dPHUojS4096ssg1aRTVnI2XqLNRAubgl9n+8PWaZ3jcsPD6JNY\njJw7NStpYynBmU9A1K3ZOTk4O7wLHQoUx9UU9M8CemrUcvh9siLc3RAhAoGAGGcg\ngDQ7j2wrpKIrB/EO+84Es2HzP3/3gtQg3OdPtaPhQdKR1aij0M1O2lLLAGpzfZf/\n5ouHjd3og0cc3GQr/0z6I5rk77sBxivBkdWP1Rveb2wnGaNWFirkGnR8DGssfk+8\nHh8LWNSRF10Ww21zCebWHwEsnefQPvJLK1pNjqECgYAHxsOV2AVLz6r3M4tkJjYq\nVIzo2ywbToKjsgX+SRkqxZJBzX43syl6tvhOHmuoYT/LKF/WFN0QvSs9BatpHqwQ\nTcWHxjiUYULgFHxiIXZCD2cMLaweYyUpDtXmXLOT2bv1Qd4glGSm/p+w71oG9Dga\nO1iD+apEEk/2MzDMH7ShKw==\n-----END PRIVATE KEY-----\n";

    fn test_identity() -> LxdIdentity {
        LxdIdentity {
            client_cert: TEST_CERT.to_string(),
            client_key:  TEST_KEY.to_string(),
            trusted:     false,
        }
    }

    #[test]
    fn pem_cert_to_der_extracts_valid_base64() {
        let der = pem_cert_to_der(TEST_CERT).unwrap();
        assert!(!der.is_empty());
        // DER-encoded X.509 certificates start with a SEQUENCE tag (0x30).
        assert_eq!(der[0], 0x30);
    }

    #[test]
    fn pem_cert_to_der_rejects_missing_markers() {
        assert!(pem_cert_to_der("not a pem").is_err());
    }

    #[tokio::test]
    async fn join_with_token_success_sends_expected_fields() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/1.0/certificates")
                .json_body_includes(r#"{ "type": "client", "name": "harvest", "trust_token": "tok-123" }"#);
            then.status(200).json_body(json!({ "type": "sync", "status": "Success", "metadata": {} }));
        });

        join_with_token(&server.base_url(), &test_identity(), "harvest", "tok-123", None, false)
            .await.unwrap();

        mock.assert();
    }

    #[tokio::test]
    async fn join_with_token_sends_der_base64_not_pem() {
        let server = MockServer::start();

        let expected_der = pem_cert_to_der(TEST_CERT).unwrap();
        let expected_b64 = STANDARD.encode(&expected_der);

        let capture = server.mock(|when, then| {
            when.method("POST")
                .path("/1.0/certificates")
                .json_body_includes(format!(r#"{{ "certificate": "{expected_b64}" }}"#));
            then.status(200).json_body(json!({ "type": "sync", "status": "Success", "metadata": {} }));
        });

        join_with_token(&server.base_url(), &test_identity(), "harvest", "tok-123", None, false)
            .await.unwrap();

        capture.assert();
    }

    #[tokio::test]
    async fn join_with_token_rejects_invalid_token() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/1.0/certificates");
            then.status(403).json_body(json!({ "error": "invalid trust token" }));
        });

        let err = join_with_token(&server.base_url(), &test_identity(), "harvest", "bad-token", None, false)
            .await.unwrap_err();
        assert!(err.to_string().contains("403"));
    }
}
