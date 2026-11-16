use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::platform::Platform;
use super::{CredentialStore, NewAccount};
use crate::auth::jwt::Claims;

pub type CredentialState = Option<Arc<CredentialStore>>;

#[derive(Deserialize)]
pub struct SshKeyBody {
    #[serde(default)]
    pub private_key: Option<String>,
}

#[derive(Deserialize)]
pub struct AccountBody {
    pub platform: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub username: String,
    pub secret: String,
}

fn failure(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn unavailable() -> Response {
    failure(
        StatusCode::SERVICE_UNAVAILABLE,
        "credential storage is not configured on this server",
    )
}

fn internal(e: anyhow::Error) -> Response {
    tracing::error!(error = %e, "credential request failed");
    failure(StatusCode::INTERNAL_SERVER_ERROR, "server error")
}

pub async fn get_credentials(
    Extension(user): Extension<Claims>,
    State(store): State<CredentialState>,
) -> Response {
    let Some(store) = store else {
        return unavailable();
    };
    let ssh_key = match store.ssh_key(&user.sub).await {
        Ok(key) => key,
        Err(e) => return internal(e),
    };
    let accounts = match store.accounts(&user.sub).await {
        Ok(accounts) => accounts,
        Err(e) => return internal(e),
    };
    let platforms: Vec<_> = Platform::ALL
        .iter()
        .map(|p| {
            json!({
                "id": p.as_str(),
                "label": p.label(),
                "default_host": p.default_host(),
                "needs_username": p.needs_username(),
            })
        })
        .collect();
    Json(json!({ "ssh_key": ssh_key, "accounts": accounts, "platforms": platforms }))
        .into_response()
}

pub async fn put_ssh_key(
    Extension(user): Extension<Claims>,
    State(store): State<CredentialState>,
    Json(body): Json<SshKeyBody>,
) -> Response {
    let Some(store) = store else {
        return unavailable();
    };
    let result = match body.private_key.as_deref().map(str::trim) {
        Some(private_key) if !private_key.is_empty() => {
            store.import_ssh_key(&user.sub, private_key).await
        }
        _ => store.generate_ssh_key(&user.sub).await,
    };
    match result {
        Ok(key) => Json(key).into_response(),
        Err(e) => failure(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
    }
}

pub async fn delete_ssh_key(
    Extension(user): Extension<Claims>,
    State(store): State<CredentialState>,
) -> Response {
    let Some(store) = store else {
        return unavailable();
    };
    match store.delete_ssh_key(&user.sub).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e),
    }
}

pub async fn put_account(
    Extension(user): Extension<Claims>,
    State(store): State<CredentialState>,
    Json(body): Json<AccountBody>,
) -> Response {
    let Some(store) = store else {
        return unavailable();
    };
    let platform: Platform = match body.platform.parse() {
        Ok(p) => p,
        Err(e) => return failure(StatusCode::BAD_REQUEST, e),
    };
    let host = body
        .host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .or(platform.default_host());
    let Some(host) = host else {
        return failure(
            StatusCode::BAD_REQUEST,
            format!("{} needs a host", platform.label()),
        );
    };
    if platform.needs_username() && body.username.trim().is_empty() {
        return failure(
            StatusCode::BAD_REQUEST,
            format!("{} needs a username", platform.label()),
        );
    }
    if body.secret.trim().is_empty() {
        return failure(StatusCode::BAD_REQUEST, "a token is required");
    }
    let account = NewAccount {
        platform,
        host,
        username: body.username.trim(),
        secret: body.secret.trim(),
    };
    match store.save_account(&user.sub, account).await {
        Ok(()) => match store.accounts(&user.sub).await {
            Ok(accounts) => Json(json!({ "accounts": accounts })).into_response(),
            Err(e) => internal(e),
        },
        Err(e) => internal(e),
    }
}

pub async fn delete_account(
    Extension(user): Extension<Claims>,
    State(store): State<CredentialState>,
    Path(host): Path<String>,
) -> Response {
    let Some(store) = store else {
        return unavailable();
    };
    match store.delete_account(&user.sub, &host).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e),
    }
}
