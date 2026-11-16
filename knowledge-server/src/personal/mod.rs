use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use harvest_db::Db;

use crate::artifacts::handlers::{create_artifact, validate_content_for_kind, ArtifactKind};
use crate::auth::jwt::Claims;
use crate::modes::Owner;
use crate::skills::http::{skill_error, SkillApiError};
use crate::skills::owned::{NewSkill, OwnedSkills, SkillChanges};

type ApiError = (StatusCode, Json<Value>);

fn failure(status: StatusCode, message: &str) -> ApiError {
    (status, Json(json!({ "error": message })))
}

fn mine(user: &Claims) -> Owner {
    Owner::User(user.sub.clone())
}

pub async fn list_artifacts(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = db
        .query(
            "SELECT id, title, kind, created_at, updated_at, created_by
             FROM artifacts WHERE user_id = $uid ORDER BY created_at DESC",
            json!({ "uid": user.sub }),
        )
        .await
        .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct NewArtifactBody {
    pub title: String,
    pub kind: String,
    pub content: String,
}

pub async fn create_personal_artifact(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
    Json(body): Json<NewArtifactBody>,
) -> Result<impl IntoResponse, ApiError> {
    let kind = ArtifactKind::parse(&body.kind)
        .ok_or_else(|| failure(StatusCode::BAD_REQUEST, "unknown artifact kind"))?;
    validate_content_for_kind(kind, &body.content)
        .map_err(|e| failure(StatusCode::BAD_REQUEST, &e))?;
    let created = create_artifact(
        &db,
        &mine(&user),
        kind,
        &body.title,
        &body.content,
        &user.sub,
    )
    .await
    .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[derive(Deserialize)]
pub struct SkillBody {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Deserialize)]
pub struct SkillPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
}

pub async fn list_skills(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
) -> Result<impl IntoResponse, SkillApiError> {
    let owner = mine(&user);
    let rows = OwnedSkills::new(&db, &owner)
        .list()
        .await
        .map_err(skill_error)?;
    Ok(Json(rows))
}

pub async fn create_skill(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
    Json(body): Json<SkillBody>,
) -> Result<impl IntoResponse, SkillApiError> {
    let owner = mine(&user);
    let skill = NewSkill {
        name: &body.name,
        description: &body.description,
        content: &body.content,
    };
    let created = OwnedSkills::new(&db, &owner)
        .create(skill, &user.sub)
        .await
        .map_err(skill_error)?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn get_skill(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, SkillApiError> {
    let owner = mine(&user);
    let row = OwnedSkills::new(&db, &owner)
        .get(&id)
        .await
        .map_err(skill_error)?;
    Ok(Json(row))
}

pub async fn update_skill(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
    Path(id): Path<String>,
    Json(body): Json<SkillPatch>,
) -> Result<impl IntoResponse, SkillApiError> {
    let owner = mine(&user);
    let changes = SkillChanges {
        name: body.name.as_deref(),
        description: body.description.as_deref(),
        content: body.content.as_deref(),
    };
    OwnedSkills::new(&db, &owner)
        .update(&id, changes)
        .await
        .map_err(skill_error)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_skill(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, SkillApiError> {
    let owner = mine(&user);
    OwnedSkills::new(&db, &owner)
        .delete(&id)
        .await
        .map_err(skill_error)?;
    Ok(StatusCode::NO_CONTENT)
}
