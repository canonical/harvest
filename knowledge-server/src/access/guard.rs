use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use harvest_db::Db;

use super::{Action, Authorizer, Caller, Denied};
use crate::auth::jwt::Claims;

const PROJECTS_PREFIX: &str = "/projects/";
const MEMBERS: &str = "members";
const INTERACTIVE_SUFFIXES: &[&str] = &["/console"];

pub fn classify(method: &Method, path: &str) -> Option<(String, Action)> {
    let rest = path.strip_prefix(PROJECTS_PREFIX)?;
    let (project_id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if project_id.is_empty() {
        return None;
    }
    let reads = matches!(*method, Method::GET | Method::HEAD);
    let action = if (tail.is_empty() && *method == Method::DELETE)
        || (tail.starts_with(MEMBERS) && !reads)
    {
        Action::Manage
    } else if INTERACTIVE_SUFFIXES.iter().any(|s| tail.ends_with(s)) || !reads {
        Action::Write
    } else {
        Action::Read
    };
    Some((project_id.to_string(), action))
}

pub async fn project_guard(State(db): State<Arc<Db>>, request: Request, next: Next) -> Response {
    let Some((project_id, action)) = classify(request.method(), request.uri().path()) else {
        return next.run(request).await;
    };
    let Some(claims) = request.extensions().get::<Claims>() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response();
    };
    let verdict = Authorizer::new(&db)
        .deploy_project(Caller::from(claims), &project_id, action)
        .await;
    match verdict {
        Ok(_) => next.run(request).await,
        Err(Denied::NotFound) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
        Err(Denied::Forbidden) => (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": format!("your role in this project does not allow this ({} required)", action.minimum_role()) })),
        )
            .into_response(),
        Err(Denied::Unavailable) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "server error" }))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(method: Method, path: &str) -> Option<Action> {
        classify(&method, path).map(|(_, action)| action)
    }

    #[test]
    fn reading_needs_a_viewer_and_changing_needs_an_editor() {
        assert_eq!(
            action(Method::GET, "/projects/p/conversations"),
            Some(Action::Read)
        );
        assert_eq!(
            action(Method::POST, "/projects/p/query/stream"),
            Some(Action::Write)
        );
        assert_eq!(
            action(Method::DELETE, "/projects/p/skills/s"),
            Some(Action::Write)
        );
    }

    #[test]
    fn opening_an_agent_console_counts_as_a_change() {
        assert_eq!(
            action(Method::GET, "/projects/p/agents/a/console"),
            Some(Action::Write)
        );
    }

    #[test]
    fn sharing_and_deleting_the_project_need_its_owner() {
        assert_eq!(
            action(Method::GET, "/projects/p/members"),
            Some(Action::Read)
        );
        assert_eq!(
            action(Method::PUT, "/projects/p/members"),
            Some(Action::Manage)
        );
        assert_eq!(
            action(Method::DELETE, "/projects/p/members/u"),
            Some(Action::Manage)
        );
        assert_eq!(action(Method::DELETE, "/projects/p"), Some(Action::Manage));
        assert_eq!(action(Method::PUT, "/projects/p"), Some(Action::Write));
    }

    #[test]
    fn the_project_list_is_not_a_project() {
        assert_eq!(classify(&Method::GET, "/projects"), None);
        assert_eq!(classify(&Method::POST, "/projects/"), None);
        assert_eq!(classify(&Method::GET, "/conversations/c"), None);
        assert_eq!(
            classify(&Method::GET, "/projects/p").map(|(id, _)| id),
            Some("p".into())
        );
    }
}
