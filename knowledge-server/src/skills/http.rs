use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use super::owned::SkillError;

pub type SkillApiError = (StatusCode, Json<Value>);

pub fn skill_error(e: SkillError) -> SkillApiError {
    let (status, message) = match e {
        SkillError::Invalid(message) => (StatusCode::BAD_REQUEST, message.to_string()),
        SkillError::NameTaken => (
            StatusCode::CONFLICT,
            "a skill with this name already exists".to_string(),
        ),
        SkillError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
        SkillError::Storage(e) => {
            tracing::error!(error = %e, "skill storage failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "server error".to_string(),
            )
        }
    };
    (status, Json(json!({ "error": message })))
}
