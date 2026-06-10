pub mod docs;
pub mod graph;
pub mod query;
pub mod repositories;
pub mod tool_description;

use axum::{
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::agent::{graph_tools, machine_tools, skill_tools, Agent};
use crate::skills::SkillRegistry;
use crate::auth::{self, handlers as auth_handlers, AuthState};
use crate::config::AuthConfig;
use crate::conversations::handlers::{self as conv_handlers, ConvState};
use crate::llm::LlmProvider;
use crate::machines::{
    handlers::{
        machines_protected_router, machines_router, MachineState,
    },
    MachineRegistry,
};
use crate::neo4j::Neo4jClient;
use crate::projects::handlers::{self as proj_handlers, ProjectState};

pub type GraphCache = RwLock<HashMap<String, Arc<String>>>;

#[derive(Clone)]
pub struct GraphState {
    pub neo4j: Arc<Neo4jClient>,
    pub cache: Arc<GraphCache>,
}

#[derive(Clone)]
pub struct QueryState {
    pub agent: Arc<Agent>,
    pub neo4j: Option<Arc<Neo4jClient>>,
}

#[derive(Clone)]
pub struct AppState {
    pub agent:            Arc<Agent>,
    pub neo4j:            Arc<Neo4jClient>,
    pub docs_dir:         Option<Arc<PathBuf>>,
    pub auth:             Arc<AuthConfig>,
    pub machine_registry: Arc<MachineRegistry>,
    pub agent_builder:    Arc<ProjectAgentBuilder>,
    pub binary_path:      Option<PathBuf>,
    pub llm:              Arc<dyn LlmProvider>,
}

#[derive(Clone)]
pub struct ProjectAgentBuilder {
    pub llm:                        Arc<dyn LlmProvider>,
    pub neo4j:                      Arc<Neo4jClient>,
    pub registry:                   Arc<MachineRegistry>,
    pub skills:                     Arc<SkillRegistry>,
    pub max_iterations:             usize,
    pub compaction_threshold_chars: usize,
    pub compaction_keep_last:       usize,
}

impl ProjectAgentBuilder {
    pub fn build(&self, project_id: String) -> Arc<Agent> {
        let mut tools = graph_tools::all_tools(Arc::clone(&self.neo4j));
        tools.push(Box::new(machine_tools::ListAgentsTool {
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(machine_tools::RunCommandTool {
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(skill_tools::ListSkillsTool {
            registry: Arc::clone(&self.skills),
        }));
        tools.push(Box::new(skill_tools::LoadSkillTool {
            registry: Arc::clone(&self.skills),
        }));
        Arc::new(
            Agent::new(Arc::clone(&self.llm), tools, self.max_iterations)
                .with_compaction(self.compaction_threshold_chars, self.compaction_keep_last),
        )
    }
}

pub fn router(state: AppState, cache: Arc<GraphCache>, server_url: String) -> Router {
    let graph_state = Arc::new(GraphState {
        neo4j: Arc::clone(&state.neo4j),
        cache,
    });

    let auth_state = Arc::new(AuthState {
        neo4j:  Arc::clone(&state.neo4j),
        config: Arc::clone(&state.auth),
        http:   reqwest::Client::new(),
    });

    let jwt_secret = Arc::new(state.auth.jwt_secret.clone());

    let conv_state = Arc::new(ConvState {
        neo4j: Arc::clone(&state.neo4j),
    });

    let public_router = Router::new()
        .route("/health", get(|| async { Json(serde_json::json!({ "status": "ok" })) }))
        .route("/auth/config",            get(auth_handlers::config))
        .route("/auth/register",          post(auth_handlers::register))
        .route("/auth/login",             post(auth_handlers::login))
        .route("/auth/logout",            post(auth_handlers::logout))
        .route("/auth/google",            get(auth_handlers::google_redirect))
        .route("/auth/google/callback",   get(auth_handlers::google_callback))
        .with_state(Arc::clone(&auth_state));

    let query_state = Arc::new(QueryState {
        agent: Arc::clone(&state.agent),
        neo4j: Some(Arc::clone(&state.neo4j)),
    });
    let agent_router = Router::new()
        .route("/query",            post(query::handle_query))
        .route("/query/stream",     post(query::handle_query_stream))
        .route("/tool-description", post(tool_description::handle_tool_description))
        .with_state(query_state);

    let graph_router = Router::new()
        .route("/repositories",                     get(repositories::handle_list_repositories))
        .route("/graph/:repo/:version",             get(graph::handle_get_graph))
        .route("/graph/:repo/:version/source",      get(graph::handle_get_symbol_source))
        .with_state(Arc::clone(&graph_state));

    let me_router = Router::new()
        .route("/auth/me", get(auth_handlers::me).patch(auth_handlers::update_me))
        .with_state(Arc::clone(&auth_state));

    let conv_router = Router::new()
        .route("/conversations",      get(conv_handlers::list).post(conv_handlers::create))
        .route("/conversations/:id",  get(conv_handlers::get)
                                     .put(conv_handlers::update)
                                     .delete(conv_handlers::delete))
        .with_state(Arc::clone(&conv_state));

    let project_state = Arc::new(ProjectState::new(
        Arc::clone(&state.neo4j),
        Arc::clone(&state.agent),
        Arc::clone(&state.agent_builder),
    ));

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
        .route("/projects/:pid/events",        get(proj_handlers::project_events))
        .route("/projects/:pid/query",         post(proj_handlers::project_query))
        .route("/projects/:pid/query/stream",  post(proj_handlers::project_query_stream))
        .route("/projects/:pid/memories",
               get(proj_handlers::list_memories).post(proj_handlers::create_memory))
        .route("/projects/:pid/memories/:mid",
               get(proj_handlers::get_memory)
               .put(proj_handlers::update_memory)
               .delete(proj_handlers::delete_memory))
        .route("/projects/:pid/tasks",
               get(proj_handlers::list_tasks).post(proj_handlers::create_task))
        .route("/projects/:pid/tasks/:tid",
               patch(proj_handlers::update_task).delete(proj_handlers::delete_task))
        .route("/projects/:pid/tasks/:tid/run",
               post(proj_handlers::run_task))
        .route("/projects/:pid/tasks/:tid/logs",
               get(proj_handlers::get_task_logs))
        .with_state(project_state);

    let machine_state = Arc::new(MachineState {
        registry:    Arc::clone(&state.machine_registry),
        neo4j:       Some(Arc::clone(&state.neo4j)),
        binary_path: state.binary_path.clone(),
        server_url,
    });

    let machines_public = machines_router(Arc::clone(&machine_state));
    let machines_protected = machines_protected_router(Arc::clone(&machine_state));

    let mut protected_router = Router::new()
        .merge(me_router)
        .merge(conv_router)
        .merge(agent_router)
        .merge(graph_router)
        .merge(project_router)
        .merge(machines_protected);

    if let Some(docs_dir) = state.docs_dir {
        let docs_router = Router::new()
            .route("/docs/:repo/:version",              get(docs::handle_get_index))
            .route("/docs/:repo/:version/:section/*filename", get(docs::handle_get_page))
            .with_state(docs_dir);
        protected_router = protected_router.merge(docs_router);
    }

    let protected_router = protected_router
        .layer(from_fn_with_state(Arc::clone(&jwt_secret), auth::require_auth));

    let admin_router = Router::new()
        .route("/admin/users",          get(crate::admin::handlers::list_users))
        .route("/admin/users/:id/role", put(crate::admin::handlers::set_user_role))
        .route("/admin/users/:id/groups", put(crate::admin::handlers::set_user_groups))
        .route("/admin/groups",         get(crate::admin::handlers::list_groups)
                                       .post(crate::admin::handlers::create_group))
        .route("/admin/groups/:id",     delete(crate::admin::handlers::delete_group))
        .with_state(Arc::clone(&auth_state))
        .layer(from_fn_with_state(Arc::clone(&jwt_secret), auth::require_admin));

    Router::new()
        .merge(public_router)
        .merge(machines_public)
        .merge(protected_router)
        .merge(admin_router)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
