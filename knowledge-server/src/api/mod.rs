pub mod docs;
pub mod graph;
pub mod query;
pub mod repositories;
pub mod tool_description;

use axum::{
    middleware::from_fn_with_state,
    routing::{delete, get, post, put},
    Json, Router,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::agent::Agent;
use crate::auth::{self, handlers as auth_handlers, AuthState};
use crate::config::AuthConfig;
use crate::conversations::handlers::{self as conv_handlers, ConvState};
use crate::neo4j::Neo4jClient;
use crate::projects::handlers::{self as proj_handlers, ProjectState};

pub type GraphCache = RwLock<HashMap<String, Arc<String>>>;

#[derive(Clone)]
pub struct GraphState {
    pub neo4j: Arc<Neo4jClient>,
    pub cache: Arc<GraphCache>,
}

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub neo4j: Arc<Neo4jClient>,
    pub docs_dir: Option<Arc<PathBuf>>,
    pub auth: Arc<AuthConfig>,
}

pub fn router(state: AppState, cache: Arc<GraphCache>) -> Router {
    let graph_state = Arc::new(GraphState {
        neo4j: Arc::clone(&state.neo4j),
        cache,
    });

    let auth_state = Arc::new(AuthState {
        neo4j: Arc::clone(&state.neo4j),
        config: Arc::clone(&state.auth),
        http: reqwest::Client::new(),
    });

    let jwt_secret = Arc::new(state.auth.jwt_secret.clone());

    let conv_state = Arc::new(ConvState {
        neo4j: Arc::clone(&state.neo4j),
    });

    let public_router = Router::new()
        .route("/health", get(|| async { Json(serde_json::json!({ "status": "ok" })) }))
        .route("/auth/config", get(auth_handlers::config))
        .route("/auth/register", post(auth_handlers::register))
        .route("/auth/login", post(auth_handlers::login))
        .route("/auth/logout", post(auth_handlers::logout))
        .route("/auth/google", get(auth_handlers::google_redirect))
        .route("/auth/google/callback", get(auth_handlers::google_callback))
        .with_state(Arc::clone(&auth_state));

    let agent_router = Router::new()
        .route("/query", post(query::handle_query))
        .route("/query/stream", post(query::handle_query_stream))
        .route("/tool-description", post(tool_description::handle_tool_description))
        .with_state(Arc::clone(&state.agent));

    let graph_router = Router::new()
        .route("/repositories", get(repositories::handle_list_repositories))
        .route("/graph/:repo/:version", get(graph::handle_get_graph))
        .route("/graph/:repo/:version/source", get(graph::handle_get_symbol_source))
        .with_state(Arc::clone(&graph_state));

    let me_router = Router::new()
        .route("/auth/me", get(auth_handlers::me))
        .with_state(Arc::clone(&auth_state));

    let conv_router = Router::new()
        .route("/conversations", get(conv_handlers::list))
        .route("/conversations", post(conv_handlers::create))
        .route("/conversations/:id", get(conv_handlers::get))
        .route("/conversations/:id", put(conv_handlers::update))
        .route("/conversations/:id", delete(conv_handlers::delete))
        .with_state(Arc::clone(&conv_state));

    let project_state = Arc::new(ProjectState {
        neo4j:  Arc::clone(&state.neo4j),
        agent:  Arc::clone(&state.agent),
    });

    let project_router = Router::new()
        .route("/groups",       get(proj_handlers::list_my_groups))
        .route("/projects",     get(proj_handlers::list_projects).post(proj_handlers::create_project))
        .route("/projects/:pid", get(proj_handlers::get_project)
                                .put(proj_handlers::update_project)
                                .delete(proj_handlers::delete_project))
        .route("/projects/:pid/conversations",
               get(proj_handlers::list_conversations).post(proj_handlers::create_conversation))
        .route("/projects/:pid/conversations/:cid",
               get(proj_handlers::get_conversation)
               .put(proj_handlers::update_conversation)
               .delete(proj_handlers::delete_conversation))
        .route("/projects/:pid/query",        post(proj_handlers::project_query))
        .route("/projects/:pid/query/stream", post(proj_handlers::project_query_stream))
        .with_state(project_state);

    let mut protected_router = Router::new()
        .merge(me_router)
        .merge(conv_router)
        .merge(agent_router)
        .merge(graph_router)
        .merge(project_router);

    if let Some(docs_dir) = state.docs_dir {
        let docs_router = Router::new()
            .route("/docs/:repo/:version", get(docs::handle_get_index))
            .route("/docs/:repo/:version/:section/*filename", get(docs::handle_get_page))
            .with_state(docs_dir);
        protected_router = protected_router.merge(docs_router);
    }

    let protected_router = protected_router
        .layer(from_fn_with_state(Arc::clone(&jwt_secret), auth::require_auth));

    let admin_router = Router::new()
        .route("/admin/users", get(crate::admin::handlers::list_users))
        .route("/admin/users/:id/role", put(crate::admin::handlers::set_user_role))
        .route("/admin/users/:id/groups", put(crate::admin::handlers::set_user_groups))
        .route("/admin/groups", get(crate::admin::handlers::list_groups))
        .route("/admin/groups", post(crate::admin::handlers::create_group))
        .route("/admin/groups/:id", delete(crate::admin::handlers::delete_group))
        .with_state(Arc::clone(&auth_state))
        .layer(from_fn_with_state(Arc::clone(&jwt_secret), auth::require_admin));

    Router::new()
        .merge(public_router)
        .merge(protected_router)
        .merge(admin_router)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
