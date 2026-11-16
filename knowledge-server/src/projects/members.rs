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

use crate::access::members::{MemberError, Members};
use crate::access::Role;
use crate::auth::jwt::Claims;

type ApiError = (StatusCode, Json<Value>);

fn failure(status: StatusCode, message: &str) -> ApiError {
    (status, Json(json!({ "error": message })))
}

fn member_error(e: MemberError) -> ApiError {
    match e {
        MemberError::UnknownUser => {
            failure(StatusCode::NOT_FOUND, "no Harvest user has this email")
        }
        MemberError::LastOwner => failure(
            StatusCode::CONFLICT,
            "a project needs at least one owner; make someone else an owner first",
        ),
        MemberError::NotMember => failure(StatusCode::NOT_FOUND, "this person is not a member"),
        MemberError::Storage(e) => {
            tracing::error!(error = %e, "project membership storage failed");
            failure(StatusCode::INTERNAL_SERVER_ERROR, "server error")
        }
    }
}

fn parse_role(role: &str) -> Result<Role, ApiError> {
    role.parse()
        .map_err(|e: String| failure(StatusCode::BAD_REQUEST, &e))
}

#[derive(Deserialize)]
pub struct AddMemberBody {
    pub email: String,
    pub role: String,
}

#[derive(Deserialize)]
pub struct RoleBody {
    pub role: String,
}

pub async fn list_members(
    State(db): State<Arc<Db>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = Members::new(&db, &project_id)
        .list()
        .await
        .map_err(member_error)?;
    Ok(Json(rows))
}

pub async fn add_member(
    Extension(user): Extension<Claims>,
    State(db): State<Arc<Db>>,
    Path(project_id): Path<String>,
    Json(body): Json<AddMemberBody>,
) -> Result<impl IntoResponse, ApiError> {
    let role = parse_role(&body.role)?;
    let members = Members::new(&db, &project_id);
    members
        .add(&body.email, role, &user.sub)
        .await
        .map_err(member_error)?;
    Ok(Json(members.list().await.map_err(member_error)?))
}

pub async fn set_member_role(
    State(db): State<Arc<Db>>,
    Path((project_id, user_id)): Path<(String, String)>,
    Json(body): Json<RoleBody>,
) -> Result<impl IntoResponse, ApiError> {
    let role = parse_role(&body.role)?;
    let members = Members::new(&db, &project_id);
    members
        .set_role(&user_id, role)
        .await
        .map_err(member_error)?;
    Ok(Json(members.list().await.map_err(member_error)?))
}

pub async fn remove_member(
    State(db): State<Arc<Db>>,
    Path((project_id, user_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    Members::new(&db, &project_id)
        .remove(&user_id)
        .await
        .map_err(member_error)?;
    Ok(StatusCode::NO_CONTENT)
}
