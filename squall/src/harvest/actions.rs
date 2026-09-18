use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

use super::client::Client;
use super::models::QueryResponse;

#[derive(Serialize)]
struct CreateProjectBody<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    group_id: &'a str,
}

#[derive(Serialize)]
struct CreateConversationBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
}

#[derive(Serialize)]
struct QueryBody<'a> {
    query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_id: Option<&'a str>,
}

#[derive(Serialize)]
struct ArtifactBody<'a> {
    title: &'a str,
    kind: &'a str,
    content: &'a Value,
}

#[derive(Serialize)]
struct GenerateDesignBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_ids: Option<&'a [String]>,
}

impl Client {
    pub async fn me(&self) -> Result<Value> {
        self.get_json("/auth/me").await
    }

    pub async fn list_groups(&self) -> Result<Vec<Value>> {
        self.get_json("/groups").await
    }

    pub async fn list_projects(&self) -> Result<Vec<Value>> {
        self.get_json("/projects").await
    }

    pub async fn create_project(&self, name: &str, description: Option<&str>, group_id: &str) -> Result<Value> {
        let body = CreateProjectBody { name, description, group_id };
        self.post_json("/projects", &body).await
    }

    pub async fn get_project(&self, project_id: &str) -> Result<Value> {
        self.get_json(&format!("/projects/{project_id}")).await
    }

    pub async fn create_conversation(&self, project_id: &str, title: Option<&str>) -> Result<Value> {
        let body = CreateConversationBody { title };
        self.post_json(&format!("/projects/{project_id}/conversations"), &body).await
    }

    pub async fn get_conversation(&self, project_id: &str, conversation_id: &str) -> Result<Value> {
        self.get_json(&format!("/projects/{project_id}/conversations/{conversation_id}")).await
    }

    pub async fn send_chat_message(&self, project_id: &str, query: &str, conversation_id: Option<&str>) -> Result<QueryResponse> {
        let body = QueryBody { query, conversation_id };
        self.post_json(&format!("/projects/{project_id}/query"), &body).await
    }

    pub async fn create_artifact(&self, project_id: &str, title: &str, kind: &str, content: &Value) -> Result<Value> {
        let body = ArtifactBody { title, kind, content };
        self.post_json(&format!("/projects/{project_id}/artifacts"), &body).await
    }

    pub async fn update_artifact(&self, artifact_id: &str, title: &str, kind: &str, content: &Value) -> Result<Value> {
        let body = ArtifactBody { title, kind, content };
        self.put_json(&format!("/artifacts/{artifact_id}"), &body).await
    }

    pub async fn get_artifact(&self, artifact_id: &str) -> Result<Value> {
        self.get_json(&format!("/artifacts/{artifact_id}")).await
    }

    pub async fn get_deployment(&self, project_id: &str) -> Result<Value> {
        self.get_json(&format!("/projects/{project_id}/deployment")).await
    }

    pub async fn generate_design(&self, project_id: &str, deployment_id: &str, artifact_ids: Option<&[String]>) -> Result<Value> {
        let body = GenerateDesignBody { artifact_ids };
        self.post_json(&format!("/projects/{project_id}/deployments/{deployment_id}/design/generate"), &body).await
    }

    pub async fn add_context_artifact(&self, project_id: &str, deployment_id: &str, title: &str, kind: &str, content: &Value) -> Result<Value> {
        let body = ArtifactBody { title, kind, content };
        self.post_json(&format!("/projects/{project_id}/deployments/{deployment_id}/context-artifacts"), &body).await
    }

    pub async fn deploy(&self, project_id: &str, deployment_id: &str) -> Result<Value> {
        self.post_json(&format!("/projects/{project_id}/deployments/{deployment_id}/deploy"), &json!({})).await
    }

    pub async fn redeploy(&self, project_id: &str, deployment_id: &str) -> Result<Value> {
        self.post_json(&format!("/projects/{project_id}/deployments/{deployment_id}/redeploy"), &json!({})).await
    }

    pub async fn destroy(&self, project_id: &str, deployment_id: &str) -> Result<Value> {
        self.post_json(&format!("/projects/{project_id}/deployments/{deployment_id}/destroy"), &json!({})).await
    }

    pub async fn list_deployment_runs(&self, project_id: &str, deployment_id: &str) -> Result<Vec<Value>> {
        self.get_json(&format!("/projects/{project_id}/deployments/{deployment_id}/runs")).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn client(server: &MockServer) -> Client {
        Client::new(server.base_url(), "tok").unwrap()
    }

    #[tokio::test]
    async fn me_gets_current_user_identity() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/auth/me");
            then.status(200).json_body(json!({"id": "u1", "email": "a@b.com", "role": "admin"}));
        });
        let resp = client(&server).me().await.unwrap();
        assert_eq!(resp["email"], "a@b.com");
    }

    #[tokio::test]
    async fn create_project_posts_name_description_and_group() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects").json_body(json!({"name": "n", "description": "d", "group_id": "g1"}));
            then.status(201).json_body(json!({"id": "p1", "name": "n"}));
        });
        let resp = client(&server).create_project("n", Some("d"), "g1").await.unwrap();
        mock.assert();
        assert_eq!(resp["id"], "p1");
    }

    #[tokio::test]
    async fn create_project_omits_description_when_none() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects").json_body(json!({"name": "n", "group_id": "g1"}));
            then.status(201).json_body(json!({"id": "p1"}));
        });
        client(&server).create_project("n", None, "g1").await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn list_projects_gets_array() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/projects");
            then.status(200).json_body(json!([{"id": "p1"}, {"id": "p2"}]));
        });
        let projects = client(&server).list_projects().await.unwrap();
        assert_eq!(projects.len(), 2);
    }

    #[tokio::test]
    async fn send_chat_message_includes_conversation_id_when_present() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/query").json_body(json!({"query": "hi", "conversation_id": "c1"}));
            then.status(200).json_body(json!({"answer": "hello"}));
        });
        let resp = client(&server).send_chat_message("p1", "hi", Some("c1")).await.unwrap();
        mock.assert();
        assert_eq!(resp.answer, "hello");
    }

    #[tokio::test]
    async fn send_chat_message_omits_conversation_id_when_none() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/query").json_body(json!({"query": "hi"}));
            then.status(200).json_body(json!({"answer": "hello"}));
        });
        client(&server).send_chat_message("p1", "hi", None).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn create_artifact_posts_title_kind_and_content() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/artifacts")
                .json_body(json!({"title": "readme", "kind": "markdown", "content": "# hi"}));
            then.status(201).json_body(json!({"id": "a1"}));
        });
        let resp = client(&server).create_artifact("p1", "readme", "markdown", &json!("# hi")).await.unwrap();
        mock.assert();
        assert_eq!(resp["id"], "a1");
    }

    #[tokio::test]
    async fn get_deployment_hits_singular_deployment_path() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/projects/p1/deployment");
            then.status(200).json_body(json!({"id": "d1", "infra_state": "none"}));
        });
        let resp = client(&server).get_deployment("p1").await.unwrap();
        assert_eq!(resp["id"], "d1");
    }

    #[tokio::test]
    async fn generate_design_posts_to_deployment_scoped_path() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/deployments/d1/design/generate");
            then.status(200).json_body(json!({"design_doc": {"id": "art1"}}));
        });
        let resp = client(&server).generate_design("p1", "d1", None).await.unwrap();
        mock.assert();
        assert_eq!(resp["design_doc"]["id"], "art1");
    }

    #[tokio::test]
    async fn add_context_artifact_posts_to_context_artifacts_path() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/deployments/d1/context-artifacts")
                .json_body(json!({"title": "notes", "kind": "markdown", "content": "hi"}));
            then.status(201).json_body(json!({"id": "d1"}));
        });
        client(&server).add_context_artifact("p1", "d1", "notes", "markdown", &json!("hi")).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn deploy_posts_to_deploy_path() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/deployments/d1/deploy");
            then.status(200).json_body(json!({"ok": true}));
        });
        client(&server).deploy("p1", "d1").await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn destroy_posts_to_destroy_path() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/deployments/d1/destroy");
            then.status(200).json_body(json!({"ok": true}));
        });
        client(&server).destroy("p1", "d1").await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn list_deployment_runs_gets_array() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/projects/p1/deployments/d1/runs");
            then.status(200).json_body(json!([{"id": "r1"}]));
        });
        let runs = client(&server).list_deployment_runs("p1", "d1").await.unwrap();
        assert_eq!(runs.len(), 1);
    }
}
