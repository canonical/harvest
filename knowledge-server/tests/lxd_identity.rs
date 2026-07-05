#[cfg(test)]
mod docker_tests {
    use std::sync::Arc;

    use httpmock::prelude::*;
    use regex::Regex;
    use serde_json::{json, Value};
    use tokio::sync::mpsc;

    use knowledge_server::config::LxdConfig;
    use knowledge_server::lxd::{self, identity, Flavor, LxdClient};
    use knowledge_server::machines::lxd_provision::create_lxd_agent;
    use knowledge_server::neo4j::Neo4jClient;
    use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j, Neo4jImageExt as _};

    async fn connect() -> Arc<Neo4jClient> {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j").to_string();
        let pass = container.image().password().unwrap_or("neo").to_string();
        let client = Neo4jClient::new(&uri, &user, &pass).await.unwrap();
        Box::leak(Box::new(container));
        Arc::new(client)
    }

    async fn seed_project(neo4j: &Neo4jClient) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        neo4j.query_read(
            "CREATE (p:Project {id: $id, name: 'test', group_id: 'g1', created_by: 'u1', created_at: '2026-01-01'})",
            json!({ "id": id }),
        ).await.unwrap();
        id
    }

    const TEST_CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIDFzCCAf+gAwIBAgIUfa1whA4eLVBTEndTkfzURCCkn1gwDQYJKoZIhvcNAQEL\nBQAwGzEZMBcGA1UEAwwQaGFydmVzdC1seGQtdGVzdDAeFw0yNjA3MDMyMDA1MzVa\nFw0zNjA2MzAyMDA1MzVaMBsxGTAXBgNVBAMMEGhhcnZlc3QtbHhkLXRlc3QwggEi\nMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC6adiI5Nwy9MDlzR92GgFc7b2L\n/ka6ccF3I6RyUxfUveDyY7WzkFOjWS6gWrM0mg+zTkP0EyEWdLaKZsZokum1gDIl\nNtd3A96d8Kz4MYxf7s+d7P+5NOyS0XaqwZmqRxkq+Ps/xrULcuBydLnkDse43+DU\n1AoueIiHZL2lDgGU0LhTUi8COln/daye2zjGv7dzaOzJCRq4YWc+D45ha8ii3GAr\natzLR8OjS9eMFrVivT5PLvGArp7qzVGvgQZ4AhTw9DELACt6gF85y4NOZ3+QhQb6\nVKza5MkNmBC6piEmOgBxOEzVwBwTw0PoQfESgGi6i2sWf5+jysCtLZAdHsSvAgMB\nAAGjUzBRMB0GA1UdDgQWBBQKFOj0BLCTRijKht/LnCciyhht1TAfBgNVHSMEGDAW\ngBQKFOj0BLCTRijKht/LnCciyhht1TAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3\nDQEBCwUAA4IBAQBTpVu67w2AzGjw2rWk32ZQp05ldBsc1PR3l/E0gxOhip4zpc3l\nOEg9KZtwtn9zkIwbRE8xDsKMY2acy1AroqcR0IUA/XXZQOXWjqKlQJRYePcIju6v\ngy636poDLDVWT09GzFEdh7adOll24i2ghRKxX+1gP6yoK7VKbh3u1K7SAPX4Cw+V\nUsTKgdy0ott1Calzr1rgLRDYxy2sAjtT98HQs+06J+JioHfkVowVBBsf8QV9Qq/d\nZCdcQ+Ej+sDvw5h5ynKUvm+LKHk6d6aKyh8cNL67Lz14znBoX/RElz/UnhSgvlc1\nklOERsYlhYR8s0qCGY8QIJfCCdOlvsuLMS5+\n-----END CERTIFICATE-----\n";
    const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQC6adiI5Nwy9MDl\nzR92GgFc7b2L/ka6ccF3I6RyUxfUveDyY7WzkFOjWS6gWrM0mg+zTkP0EyEWdLaK\nZsZokum1gDIlNtd3A96d8Kz4MYxf7s+d7P+5NOyS0XaqwZmqRxkq+Ps/xrULcuBy\ndLnkDse43+DU1AoueIiHZL2lDgGU0LhTUi8COln/daye2zjGv7dzaOzJCRq4YWc+\nD45ha8ii3GAratzLR8OjS9eMFrVivT5PLvGArp7qzVGvgQZ4AhTw9DELACt6gF85\ny4NOZ3+QhQb6VKza5MkNmBC6piEmOgBxOEzVwBwTw0PoQfESgGi6i2sWf5+jysCt\nLZAdHsSvAgMBAAECggEAMl92zWs2m6hq1c5LoaDeXGu77C/+kdQ6iMS/Y8tTZcAX\noLhT+d1W1I29XUSVJ3I4KuZL05E1wDkyuIyUMd79O3gUVN0QdU884WYPf5P4EFZa\nkRzhb30/Ll9e1z6wlQRYZzXXwwChnKHix9sF/nwF+U26FhjkVXFpx1hwLMFvqPQV\nozsQGU2yxLpK1BU8p6peSYcbNy/klCizHdFrUQfI8YP4Vx6a1S7An1C7S857Xcfz\nH/QA8w7RZrW5zNznIyXM+gPC3nzY45FRBXKP8QVirYJi+DaJPCwfH9Kb7Vm090dr\nAMh9koJa1pY75bv7e6PDw1JRhRfbKBDnkn27gtY8qQKBgQDksHSZAvVJQMvPSXwr\nI+SUwDquE1hH72iRGpk3Fv1KuBqDQXvK+ITY6SL2T0VjRVFZEaX6iaPQJ8nyxBpc\neGYi7Y+UqlCN++O8Kze+yMTsQcbL5xzs2rZnZS3Sc4JMJMgn2f8+9GrPFTuSId0h\nxEGTBiG2HwPZ2J+JXwVglKp/aQKBgQDQrOx4aJwuOHyNjibRX1zjkv1f41yB+lrz\nBxI0phmNNr50Gk9fez0+CIokZbkGmHKj7K0ctR0siqKC7UyK/Zuj1ym0z6aRXhzm\nQ5lORHcA7ODYRSlYiEZUeamZI32gCnPB4NDlpod6Yqtr5b/iNjHy9/vrZnevck9S\n7JA41Yq4VwKBgB3+1wxKywl0qkbiCJtP9edc31V9zBKDYF/H8Vi8dzSZuUCGEkqp\nFiOtUJymAR/oM6dPHUojS4096ssg1aRTVnI2XqLNRAubgl9n+8PWaZ3jcsPD6JNY\njJw7NStpYynBmU9A1K3ZOTk4O7wLHQoUx9UU9M8CemrUcvh9siLc3RAhAoGAGGcg\ngDQ7j2wrpKIrB/EO+84Es2HzP3/3gtQg3OdPtaPhQdKR1aij0M1O2lLLAGpzfZf/\n5ouHjd3og0cc3GQr/0z6I5rk77sBxivBkdWP1Rveb2wnGaNWFirkGnR8DGssfk+8\nHh8LWNSRF10Ww21zCebWHwEsnefQPvJLK1pNjqECgYAHxsOV2AVLz6r3M4tkJjYq\nVIzo2ywbToKjsgX+SRkqxZJBzX43syl6tvhOHmuoYT/LKF/WFN0QvSs9BatpHqwQ\nTcWHxjiUYULgFHxiIXZCD2cMLaweYyUpDtXmXLOT2bv1Qd4glGSm/p+w71oG9Dga\nO1iD+apEEk/2MzDMH7ShKw==\n-----END PRIVATE KEY-----\n";

    fn base_cfg(endpoint: String) -> LxdConfig {
        LxdConfig {
            endpoint,
            client_cert:  None,
            client_key:   None,
            trust_token:  None,
            ca_cert:      None,
            insecure:     false,
            project:      "default".into(),
            image_alias:  "24.04".into(),
            image_server: "https://cloud-images.ubuntu.com/releases".into(),
            profile:      "default".into(),
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn load_or_generate_creates_identity_when_absent() {
        let neo4j = connect().await;

        let identity = identity::load_or_generate(&neo4j).await.unwrap();
        assert!(identity.client_cert.contains("BEGIN CERTIFICATE"));
        assert!(identity.client_key.contains("PRIVATE KEY"));
        assert!(!identity.trusted);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn load_or_generate_is_idempotent() {
        let neo4j = connect().await;

        let first = identity::load_or_generate(&neo4j).await.unwrap();
        let second = identity::load_or_generate(&neo4j).await.unwrap();

        assert_eq!(first.client_cert, second.client_cert);
        assert_eq!(first.client_key, second.client_key);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn mark_trusted_flips_the_flag() {
        let neo4j = connect().await;

        let generated = identity::load_or_generate(&neo4j).await.unwrap();
        assert!(!generated.trusted);

        identity::mark_trusted(&neo4j).await.unwrap();

        let reloaded = identity::load_or_generate(&neo4j).await.unwrap();
        assert!(reloaded.trusted);
        assert_eq!(reloaded.client_cert, generated.client_cert);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn resolve_client_manual_cert_skips_identity_generation() {
        let neo4j = connect().await;
        let mut cfg = base_cfg("https://lxd.example.com:8443".into());
        cfg.client_cert = Some(TEST_CERT.to_string());
        cfg.client_key  = Some(TEST_KEY.to_string());

        let client = lxd::resolve_client(&cfg, &neo4j).await.unwrap();
        assert!(client.is_some());

        let rows = neo4j.query_read(
            "MATCH (i:LxdIdentity) RETURN i.id AS id", json!({}),
        ).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn resolve_client_without_token_returns_none() {
        let neo4j = connect().await;
        let cfg = base_cfg("https://lxd.example.com:8443".into());

        let client = lxd::resolve_client(&cfg, &neo4j).await.unwrap();
        assert!(client.is_none());

        let ident = identity::load_or_generate(&neo4j).await.unwrap();
        assert!(!ident.trusted, "identity should still be untrusted with no token supplied");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn resolve_client_joins_via_valid_token_and_persists_trusted() {
        let neo4j = connect().await;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/1.0/certificates");
            then.status(200).json_body(json!({ "type": "sync", "status": "Success", "metadata": {} }));
        });

        let mut cfg = base_cfg(server.base_url());
        cfg.trust_token = Some("tok-123".into());

        let client = lxd::resolve_client(&cfg, &neo4j).await.unwrap();
        assert!(client.is_some());

        let ident = identity::load_or_generate(&neo4j).await.unwrap();
        assert!(ident.trusted);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn resolve_client_rejected_token_returns_none_and_stays_untrusted() {
        let neo4j = connect().await;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/1.0/certificates");
            then.status(403).json_body(json!({ "error": "invalid trust token" }));
        });

        let mut cfg = base_cfg(server.base_url());
        cfg.trust_token = Some("bad-token".into());

        let client = lxd::resolve_client(&cfg, &neo4j).await.unwrap();
        assert!(client.is_none());

        let ident = identity::load_or_generate(&neo4j).await.unwrap();
        assert!(!ident.trusted);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn create_lxd_agent_emits_error_event_when_network_fails() {
        let neo4j = connect().await;
        let project_id = seed_project(&neo4j).await;

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path_matches(Regex::new(r"^/1\.0/networks/").unwrap());
            then.status(500).json_body(json!({ "error": "boom" }));
        });

        let lxd = LxdClient::from_identity(TEST_CERT, TEST_KEY, &base_cfg(server.base_url())).unwrap();
        let (tx, mut rx) = mpsc::channel::<String>(64);

        let result = create_lxd_agent(
            &neo4j, &lxd, "http://localhost:8080", &project_id, "Test Agent", "desc", Flavor::Small, tx,
        ).await;

        assert!(result.is_err());

        let mut events = Vec::new();
        while let Ok(data) = rx.try_recv() {
            events.push(serde_json::from_str::<Value>(&data).unwrap());
        }

        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "phase_start");
        assert_eq!(events[0]["phase"], "ensure_network");
        assert_eq!(events[1]["type"], "error");
        assert_eq!(events[1]["phase"], "ensure_network");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn create_lxd_agent_emits_expected_phase_sequence_on_success() {
        let neo4j = connect().await;
        let project_id = seed_project(&neo4j).await;

        let instance_path = Regex::new(r"^/1\.0/instances/test-agent-[0-9a-f]{4}$").unwrap();
        let instance_state_path = Regex::new(r"^/1\.0/instances/test-agent-[0-9a-f]{4}/state$").unwrap();
        let instance_exec_path = Regex::new(r"^/1\.0/instances/test-agent-[0-9a-f]{4}/exec$").unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path_matches(Regex::new(r"^/1\.0/networks/").unwrap());
            then.status(404).json_body(json!({ "error": "not found" }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/1.0/networks");
            then.status(200).json_body(json!({ "type": "sync", "status": "Success", "metadata": {} }));
        });
        server.mock(|when, then| {
            when.method("POST").path("/1.0/instances");
            then.status(202).json_body(json!({ "type": "async", "metadata": { "id": "op-create" } }));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-create/wait");
            then.status(200).json_body(json!({
                "type": "sync",
                "metadata": { "id": "op-create", "status": "Success", "err": "", "metadata": {} },
            }));
        });
        let instance_state_path_put = instance_state_path.clone();
        server.mock(move |when, then| {
            when.method("PUT").path_matches(instance_state_path_put.clone());
            then.status(202).json_body(json!({ "type": "async", "metadata": { "id": "op-state" } }));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-state/wait");
            then.status(200).json_body(json!({
                "type": "sync",
                "metadata": { "id": "op-state", "status": "Success", "err": "", "metadata": {} },
            }));
        });
        server.mock(move |when, then| {
            when.method("GET").path_matches(instance_state_path.clone());
            then.status(200).json_body(json!({ "type": "sync", "metadata": { "status": "Running" } }));
        });
        server.mock(move |when, then| {
            when.method("POST").path_matches(instance_exec_path.clone());
            then.status(202).json_body(json!({ "type": "async", "metadata": { "id": "op-exec" } }));
        });
        server.mock(|when, then| {
            when.method("GET").path("/1.0/operations/op-exec/wait");
            then.status(200).json_body(json!({
                "type": "sync",
                "metadata": { "id": "op-exec", "status": "Success", "err": "", "metadata": { "return": 0 } },
            }));
        });
        server.mock(move |when, then| {
            when.method("DELETE").path_matches(instance_path.clone());
            then.status(404).json_body(json!({ "error": "not found" }));
        });

        let lxd = LxdClient::from_identity(TEST_CERT, TEST_KEY, &base_cfg(server.base_url())).unwrap();
        let (tx, mut rx) = mpsc::channel::<String>(64);

        let neo4j2 = Arc::clone(&neo4j);
        let project_id2 = project_id.clone();
        let handle = tokio::spawn(async move {
            create_lxd_agent(
                &neo4j2, &lxd, "http://localhost:8080", &project_id2, "Test Agent", "desc", Flavor::Small, tx,
            ).await
        });

        let mut events = Vec::new();
        while let Some(data) = rx.recv().await {
            events.push(serde_json::from_str::<Value>(&data).unwrap());
        }
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "{result:?}");

        let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(types, vec![
            "phase_start", "phase_start", "phase_start", "phase_start", "phase_start", "phase_start", "done",
        ]);

        let phases: Vec<&str> = events.iter()
            .filter(|e| e["type"] == "phase_start")
            .map(|e| e["phase"].as_str().unwrap())
            .collect();
        assert_eq!(phases, vec![
            "ensure_network", "install_token", "create_container",
            "start_container", "wait_running", "install_agent",
        ]);

        assert!(events.last().unwrap()["hostname"].as_str().unwrap().starts_with("test-agent-"));
    }
}
