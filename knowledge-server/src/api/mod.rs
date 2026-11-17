pub mod api_keys;
pub mod cost;
pub mod docs;
pub mod external_chat;
pub mod graph;
pub mod health;
pub mod llm;
pub mod query;
pub mod repositories;
pub mod tool_description;
pub mod turn_hooks;

use axum::{
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::agent::roles::Role;
use crate::agent::{deployment_tools, graph_tools, prompt, tool, Agent};
use crate::artifacts::handlers::{self as artifact_handlers, ArtifactState};
use crate::auth::api_keys::ApiKeyStore;
use crate::auth::user_keys::UserKeyStore;
use crate::auth::{self, handlers as auth_handlers, tui as auth_tui, AuthState};
use crate::chat_layouts::handlers::{self as chat_layout_handlers, ChatLayoutState};
use crate::config::AuthConfig;
use crate::config::{LlmProviderConfig, UiConfig};
use crate::conversations::handlers::{self as conv_handlers, ConvState};
use crate::credentials::{handlers as credential_handlers, CredentialStore};
use crate::deployments::{self, handlers as deployment_handlers};
use crate::ingestion::IngestionRegistry;
use crate::llm::LlmProvider;
use crate::lxd::LxdClient;
use crate::machines::{
    handlers::{machines_protected_router, machines_router, MachineState},
    MachineRegistry,
};
use crate::modes::Space;
use crate::projects::handlers::{self as proj_handlers, ProjectState};
use crate::skills::{handlers as skill_handlers, SkillStore};
use crate::toolsets::{Toolbox, ToolsetId, TurnContext};
use harvest_db::Db;

pub type GraphCache = RwLock<HashMap<String, Arc<String>>>;

pub async fn resolve_user_llm(
    llm: &Arc<dyn LlmProvider>,
    configs: &[LlmProviderConfig],
    user_key_store: &Option<Arc<UserKeyStore>>,
    user_id: &str,
) -> Arc<dyn LlmProvider> {
    let has_user_key_providers = configs.iter().any(|c| c.user_provided_key());
    if !has_user_key_providers {
        return Arc::clone(llm);
    }
    let Some(store) = user_key_store else {
        return Arc::clone(llm);
    };
    match store.resolve_all(user_id).await {
        Ok(keys) if !keys.is_empty() => crate::llm::with_user_keys(llm, configs, &keys),
        _ => Arc::clone(llm),
    }
}

pub async fn resolve_user_system_one(
    config: &Option<crate::config::SystemOneConfig>,
    llm_configs: &[LlmProviderConfig],
    user_key_store: &Option<Arc<UserKeyStore>>,
    user_id: &str,
) -> Option<Arc<crate::llm::system_one::SystemOneClient>> {
    let so_config = config.as_ref()?;
    if !so_config.is_enabled() {
        return None;
    }
    crate::llm::system_one::SystemOneClient::resolve_for_user(
        so_config,
        llm_configs,
        user_key_store,
        user_id,
    )
    .await
}

#[derive(Clone)]
pub struct GraphState {
    pub db: Arc<Db>,
    pub credentials: Option<Arc<CredentialStore>>,
    pub cache: Arc<GraphCache>,
    pub ingestion: IngestionRegistry,
    pub bus: Arc<crate::cluster::bus::ClusterBus>,
}

#[derive(Clone)]
pub struct QueryState {
    pub thresholds: Option<Thresholds>,
    pub system_one: Option<Arc<crate::llm::system_one::SystemOneClient>>,
    pub db: Option<Arc<Db>>,
    pub llm: Arc<dyn LlmProvider>,
    pub llm_configs: Arc<Vec<LlmProviderConfig>>,
    pub user_key_store: Option<Arc<UserKeyStore>>,
    pub max_iterations: usize,
    pub compaction_threshold_chars: usize,
    pub compaction_keep_last: usize,
    pub pricing: Arc<crate::cost::PricingTable>,
    pub toolbox: Arc<Toolbox>,
    pub admission: Arc<crate::turns::admission::TurnAdmission>,
    pub code: Option<crate::code::CodeEnv>,
    pub turn_hooks: Option<Arc<dyn turn_hooks::TurnHooks>>,
}

impl QueryState {
    async fn briefing_for(
        &self,
        space: &Space,
        conversation_id: &str,
        reader: crate::code::briefing::Reader,
    ) -> Option<crate::code::briefing::Briefing> {
        if space.mode() != crate::modes::Mode::Code {
            return None;
        }
        Some(match (&self.code, space.code_session_id()) {
            (Some(env), Some(session_id)) => {
                crate::code::briefing::for_session(env, session_id, conversation_id, reader).await
            }
            _ => crate::code::briefing::Briefing::without_session(),
        })
    }

    pub async fn agent_in(
        &self,
        space: Space,
        user_id: &str,
        conversation_id: Option<&str>,
        llm: Arc<dyn LlmProvider>,
    ) -> Arc<Agent> {
        self.build_agent(space, user_id, conversation_id.unwrap_or_default(), llm, None)
            .await
    }

    pub async fn agent_for_role(
        &self,
        space: Space,
        user_id: &str,
        conversation_id: &str,
        llm: Arc<dyn LlmProvider>,
        role: &'static Role,
    ) -> Arc<Agent> {
        self.build_agent(space, user_id, conversation_id, llm, Some(role)).await
    }

    async fn build_agent(
        &self,
        space: Space,
        user_id: &str,
        conversation_id: &str,
        llm: Arc<dyn LlmProvider>,
        role: Option<&'static Role>,
    ) -> Arc<Agent> {
        let reader = match role {
            Some(_) => crate::code::briefing::Reader::Reviewer,
            None => crate::code::briefing::Reader::Author,
        };
        let briefing = self.briefing_for(&space, conversation_id, reader).await;
        let turn = match &self.db {
            Some(db) => TurnContext::load(db, user_id, space, conversation_id, llm).await,
            None => TurnContext::unrestricted(user_id, space, conversation_id, llm),
        }
        .focused_on(briefing.as_ref().and_then(|b| b.focus.clone()));
        let tools: Vec<_> = self
            .toolbox
            .tools(&turn)
            .into_iter()
            .filter(|tool| role.is_none_or(|r| r.allows(&tool.definition().name)))
            .collect();
        let tools = match (&self.code, turn.space.code_session_id(), role) {
            (Some(env), Some(session_id), None) => {
                crate::code::jobs::reporting::JobReports::wrap(tools, env, session_id, conversation_id)
            }
            _ => tools,
        };
        let appendix = briefing.as_ref().map(|b| match role {
            Some(role) => b.appendix_for(role.instructions),
            None => b.appendix(),
        });
        let agent = Agent::new(
            Arc::clone(&turn.llm),
            tools,
            role.map_or(self.max_iterations, |r| r.max_iterations),
        )
        .with_system_one(self.system_one.clone())
        .with_compaction(self.compaction_threshold_chars, self.compaction_keep_last)
        .with_parallel_research(role.is_none() && briefing.as_ref().is_none_or(|b| b.parallel_research))
        .with_prompt_appendix(appendix.as_deref());
        Arc::new(match self.thresholds {
            Some(t) => t.apply(agent),
            None => agent,
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub db: Arc<Db>,
    pub ingestion: IngestionRegistry,
    pub docs_dir: Option<Arc<PathBuf>>,
    pub auth: Arc<AuthConfig>,
    pub ui: Arc<UiConfig>,
    pub machine_registry: Arc<MachineRegistry>,
    pub agent_builder: Arc<ProjectAgentBuilder>,
    pub binary_path: Option<PathBuf>,
    pub llm: Arc<dyn LlmProvider>,
    pub llm_configs: Arc<Vec<LlmProviderConfig>>,
    pub system_one_config: Option<crate::config::SystemOneConfig>,
    pub user_key_store: Option<Arc<UserKeyStore>>,
    pub api_key_store: Arc<ApiKeyStore>,
    pub lxd: Option<Arc<LxdClient>>,
    pub collocate_registry: Arc<crate::collocate::sessions::SessionContainerRegistry>,
    pub pricing: Arc<crate::cost::PricingTable>,
    pub semantic: Option<Arc<graph_tools::SemanticHandle>>,
    pub live: Arc<crate::projects::live::ProjectLive>,
    pub admission: Arc<crate::turns::admission::TurnAdmission>,
    pub credentials: Option<Arc<CredentialStore>>,
    pub code: Option<crate::code::CodeEnv>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub fast_path: f64,
    pub early_synthesis: f64,
    pub relevance: f64,
    pub early_synthesis_research: f64,
    pub relevance_preserve_recent: usize,
    pub early_synthesis_coverage: f64,
    pub early_synthesis_min_iterations_first_turn: usize,
    pub early_synthesis_uniform: f64,
    pub early_synthesis_capability_gate: f64,
    pub next_action_confidence: f64,
}

impl Thresholds {
    pub fn from_system_one(config: Option<&crate::config::SystemOneConfig>) -> Self {
        match config {
            Some(so) if so.is_enabled() => Self {
                fast_path: so.fast_path,
                early_synthesis: so.early_synthesis,
                relevance: so.relevance,
                early_synthesis_research: so.early_synthesis_research,
                relevance_preserve_recent: so.relevance_preserve_recent,
                early_synthesis_coverage: so.early_synthesis_coverage,
                early_synthesis_min_iterations_first_turn: so
                    .early_synthesis_min_iterations_first_turn,
                early_synthesis_uniform: so.early_synthesis_uniform,
                early_synthesis_capability_gate: so.early_synthesis_capability_gate,
                next_action_confidence: so.next_action_confidence,
            },
            _ => Self {
                fast_path: 0.7,
                early_synthesis: 0.85,
                relevance: 0.4,
                early_synthesis_research: 0.95,
                relevance_preserve_recent: 2,
                early_synthesis_coverage: 0.8,
                early_synthesis_min_iterations_first_turn: 5,
                early_synthesis_uniform: 0.7,
                early_synthesis_capability_gate: 0.8,
                next_action_confidence: 0.5,
            },
        }
    }

    pub fn apply(self, agent: Agent) -> Agent {
        agent
            .with_thresholds(
                self.fast_path,
                self.early_synthesis,
                self.relevance,
                self.early_synthesis_research,
                self.relevance_preserve_recent,
                self.early_synthesis_coverage,
                self.early_synthesis_min_iterations_first_turn,
                self.early_synthesis_uniform,
            )
            .with_capability_gate_threshold(self.early_synthesis_capability_gate)
            .with_next_action_confidence(self.next_action_confidence)
    }
}

#[derive(Clone)]
pub struct ProjectAgentBuilder {
    pub llm: Arc<dyn LlmProvider>,
    pub system_one: Option<Arc<crate::llm::system_one::SystemOneClient>>,
    pub db: Arc<Db>,
    pub registry: Arc<MachineRegistry>,
    pub max_iterations: usize,
    pub compaction_threshold_chars: usize,
    pub compaction_keep_last: usize,
    pub thresholds: Option<Thresholds>,
    pub search: Option<Arc<crate::search::SearchService>>,
    pub toolbox: Arc<Toolbox>,
}

pub fn code_tools(
    db: &Arc<Db>,
    semantic: Option<Arc<graph_tools::SemanticHandle>>,
    search: Option<&Arc<crate::search::SearchService>>,
    context: crate::search::code_tool::SearchToolContext,
) -> Vec<Box<dyn tool::Tool>> {
    match search {
        Some(service) if service.mode() == crate::config::SearchMode::Tiered => {
            crate::search::code_tool::tiered_tools(Arc::clone(db), Arc::clone(service), context)
        }
        _ => graph_tools::all_tools_with_semantic(Arc::clone(db), semantic),
    }
}

impl ProjectAgentBuilder {
    fn finish(&self, agent: Agent) -> Arc<Agent> {
        let agent =
            agent.with_compaction(self.compaction_threshold_chars, self.compaction_keep_last);
        match self.thresholds {
            Some(t) => Arc::new(t.apply(agent)),
            None => Arc::new(agent),
        }
    }

    pub fn conversation_agent(
        &self,
        turn: &TurnContext,
        system_one: Option<Arc<crate::llm::system_one::SystemOneClient>>,
    ) -> Arc<Agent> {
        let agent = Agent::new(
            Arc::clone(&turn.llm),
            self.toolbox.tools(turn),
            self.max_iterations,
        )
        .with_system_one(system_one)
        .with_parallel_research(true);
        self.finish(agent)
    }

    pub fn deployment_agent(
        &self,
        turn: &TurnContext,
        ctx: &deployments::DeploymentContext,
    ) -> Arc<Agent> {
        let project_id = turn
            .space
            .deploy_project_id()
            .unwrap_or_default()
            .to_string();
        let mut tools = self.toolbox.tools(turn);
        tools.push(Box::new(deployment_tools::LinkDeploymentArtifactTool {
            db: Arc::clone(&self.db),
            project_id: project_id.clone(),
            deployment_id: ctx.deployment_id.clone(),
        }));
        tools.push(Box::new(deployment_tools::UpdateProductTemplateTool {
            db: Arc::clone(&self.db),
            deployment_id: ctx.deployment_id.clone(),
        }));
        tools.push(Box::new(deployment_tools::SetExecutionPlanTool {
            db: Arc::clone(&self.db),
            project_id,
            deployment_id: ctx.deployment_id.clone(),
        }));
        let agent = Agent::new(Arc::clone(&turn.llm), tools, self.max_iterations)
            .with_system_prompt(prompt::deployment_system_prompt(ctx));
        self.finish(agent)
    }

    pub fn deployment_text_agent(
        &self,
        llm: Arc<dyn LlmProvider>,
        ctx: &deployments::DeploymentContext,
    ) -> Arc<Agent> {
        let agent = Agent::new(llm, Vec::new(), self.max_iterations)
            .with_system_prompt(prompt::deployment_system_prompt(ctx));
        self.finish(agent)
    }

    pub fn deployment_design_agent(
        &self,
        turn: &TurnContext,
        ctx: &deployments::DeploymentContext,
    ) -> Arc<Agent> {
        let tools = self
            .toolbox
            .tools_from(&[ToolsetId::Knowledge, ToolsetId::Skills], turn);
        let agent = Agent::new(Arc::clone(&turn.llm), tools, self.max_iterations)
            .with_system_prompt(prompt::deployment_system_prompt(ctx));
        self.finish(agent)
    }
}

pub async fn router(state: AppState, cache: Arc<GraphCache>, server_url: String) -> Router {
    let graph_state = Arc::new(GraphState {
        db: Arc::clone(&state.db),
        credentials: state.credentials.clone(),
        cache,
        ingestion: state.ingestion.clone(),
        bus: Arc::clone(state.live.bus()),
    });

    let http = reqwest::Client::new();
    let oidc_endpoints = if let Some(oidc_cfg) = state.auth.oidc.as_ref() {
        match auth::oidc::discover_endpoints(&http, &oidc_cfg.issuer_url).await {
            Ok(ep) => {
                tracing::info!(issuer = %oidc_cfg.issuer_url, "OIDC endpoints discovered");
                Some(Arc::new(ep))
            }
            Err(e) => {
                tracing::warn!(error = %e, "OIDC discovery failed; OIDC login will be unavailable");
                None
            }
        }
    } else {
        None
    };
    let auth_state = Arc::new(AuthState {
        db: Arc::clone(&state.db),
        config: Arc::clone(&state.auth),
        ui: Arc::clone(&state.ui),
        http,
        oidc_endpoints,
        lxd_enabled: state.lxd.is_some(),
    });

    let jwt_secret = Arc::new(state.auth.jwt_secret.clone());

    let health_state = Arc::new(health::HealthState {
        db: Arc::clone(&state.db),
        live: Arc::clone(&state.live),
        registry: Arc::clone(&state.machine_registry),
        admission: Arc::clone(&state.admission),
    });

    let conv_state = Arc::new(ConvState {
        db: Arc::clone(&state.db),
    });

    let chat_layout_state = Arc::new(ChatLayoutState {
        db: Arc::clone(&state.db),
    });

    let public_router = Router::new()
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .route(
            "/health/ready",
            get(health::ready).with_state(Arc::clone(&health_state)),
        )
        .route(
            "/version",
            get(health::version).with_state(Arc::clone(&health_state)),
        )
        .route(
            "/metrics",
            get(health::metrics).with_state(Arc::clone(&health_state)),
        )
        .route("/auth/config", get(auth_handlers::config))
        .route("/auth/register", post(auth_handlers::register))
        .route("/auth/login", post(auth_handlers::login))
        .route("/auth/logout", post(auth_handlers::logout))
        .route("/auth/google", get(auth_handlers::google_redirect))
        .route("/auth/google/callback", get(auth_handlers::google_callback))
        .route("/auth/oidc", get(auth_handlers::oidc_redirect))
        .route("/auth/oidc/callback", get(auth_handlers::oidc_callback))
        .route("/auth/tui/login", post(auth_tui::create_auth_request))
        .route("/auth/tui/poll/:uuid", get(auth_tui::poll_auth_request))
        .with_state(Arc::clone(&auth_state));

    let query_state = Arc::new(QueryState {
        thresholds: state.agent_builder.thresholds,
        system_one: state.agent_builder.system_one.clone(),
        db: Some(Arc::clone(&state.db)),
        llm: Arc::clone(&state.llm),
        llm_configs: Arc::clone(&state.llm_configs),
        user_key_store: state.user_key_store.clone(),
        max_iterations: state.agent_builder.max_iterations,
        compaction_threshold_chars: state.agent_builder.compaction_threshold_chars,
        compaction_keep_last: state.agent_builder.compaction_keep_last,
        pricing: Arc::clone(&state.pricing),
        toolbox: Arc::clone(&state.agent_builder.toolbox),
        admission: Arc::clone(&state.admission),
        code: state.code.clone(),
        turn_hooks: state
            .code
            .clone()
            .map(|env| Arc::new(turn_hooks::CodeTurnHooks { env }) as Arc<dyn turn_hooks::TurnHooks>),
    });
    let agent_router = Router::new()
        .route("/query", post(query::handle_query))
        .route("/query/stream", post(query::handle_query_stream))
        .route(
            "/tool-description",
            post(tool_description::handle_tool_description),
        )
        .with_state(Arc::clone(&query_state));

    let external_chat_state = Arc::new(external_chat::ExternalChatState {
        qs: Arc::clone(&query_state),
    });

    let graph_router = Router::new()
        .route(
            "/repositories",
            get(repositories::handle_list_repositories).post(repositories::handle_add_repository),
        )
        .route(
            "/repositories/verify",
            post(repositories::handle_verify_repository),
        )
        .route(
            "/repositories/:name/progress",
            get(repositories::handle_repository_progress),
        )
        .route(
            "/repositories/:name/stats",
            get(repositories::handle_get_repository_stats),
        )
        .route(
            "/repositories/:name/versions",
            post(repositories::handle_ingest_versions),
        )
        .route(
            "/repositories/:name/versions/:version/resync",
            post(repositories::handle_resync_version),
        )
        .route("/graph/:repo/:version", get(graph::handle_get_graph))
        .route(
            "/graph/:repo/:version/source",
            get(graph::handle_get_symbol_source),
        )
        .with_state(Arc::clone(&graph_state));

    let llm_state = Arc::new(llm::LlmState::new(
        Arc::clone(&state.llm),
        Arc::clone(&state.llm_configs),
        state.user_key_store.clone(),
        state.system_one_config.clone(),
    ));
    let llm_router = Router::new()
        .route("/llm/providers", get(llm::list_providers))
        .route("/llm/user-keys", get(llm::get_user_keys))
        .route(
            "/llm/user-keys/:provider_id",
            put(llm::set_user_key).delete(llm::delete_user_key),
        )
        .with_state(llm_state);

    let me_router = Router::new()
        .route(
            "/auth/me",
            get(auth_handlers::me).patch(auth_handlers::update_me),
        )
        .route(
            "/auth/tui/authorize/:uuid",
            post(auth_tui::authorize_request),
        )
        .with_state(Arc::clone(&auth_state));

    let credential_router = Router::new()
        .route("/me/credentials", get(credential_handlers::get_credentials))
        .route(
            "/me/ssh-key",
            put(credential_handlers::put_ssh_key).delete(credential_handlers::delete_ssh_key),
        )
        .route("/me/accounts", put(credential_handlers::put_account))
        .route(
            "/me/accounts/:host",
            delete(credential_handlers::delete_account),
        )
        .with_state(state.credentials.clone());

    let code_state = Arc::new(crate::code::handlers::CodeState {
        env: state.code.clone(),
        graph: Arc::clone(&graph_state),
        query: Arc::clone(&query_state),
    });
    let code_router = {
        use crate::code::chat_handlers as chat;
        use crate::code::handlers as code;
        use crate::code::job_handlers as jobs;
        use crate::code::previews::owner;
        Router::new()
            .route("/code/projects", get(code::list_projects).post(code::open_project))
            .route("/code/projects/:id", get(code::get_project))
            .route("/code/projects/:id/setup", put(code::set_setup))
            .route("/code/projects/:id/checks", put(jobs::set_checks))
            .route("/code/sessions/:id/jobs", get(jobs::list))
            .route("/code/sessions/:id/checks", post(jobs::run_checks))
            .route("/code/jobs/:id", get(jobs::get))
            .route("/code/jobs/:id/stop", post(jobs::stop))
            .route("/code/projects/:id/sessions", post(code::open_session))
            .route(
                "/code/sessions/:id",
                get(code::get_session).delete(code::close_session),
            )
            .route("/code/sessions/:id/changes", get(code::session_changes))
            .route("/code/sessions/:id/change-request", post(code::propose_change))
            .route("/code/sessions/:id/chats", get(chat::list).post(chat::create))
            .route("/code/sessions/:id/chats/:chat", delete(chat::delete).patch(chat::rename))
            .route("/code/sessions/:id/layout", get(chat::layout).put(chat::save_layout))
            .route("/code/sessions/:id/previews", get(owner::list).post(owner::create))
            .route("/code/previews/:id", delete(owner::delete))
            .route("/code/previews/:id/share", put(owner::share))
            .route("/code/previews/:id/restart", post(owner::restart))
            .route("/code/previews/:id/log", get(owner::log))
            .route("/code/previews/:id/keep", post(owner::keep))
            .route("/code/sessions/:id/query/stream", post(code::session_query_stream))
            .with_state(code_state)
    };

    let preview_passes = Arc::new(crate::code::previews::pass::Passes::new(&jwt_secret));
    let preview_origin = state.code.as_ref().and_then(|env| env.settings.preview_origin.clone());
    let preview_site = state.code.clone().zip(preview_origin.clone()).map(|(env, origin)| {
        let site = crate::code::previews::site::Site::new(env, origin.clone(), Arc::clone(&preview_passes), server_url.clone());
        (crate::code::previews::site::router(Arc::new(site)), origin)
    });
    let preview_router = {
        use crate::code::previews::handlers as preview;
        Router::new()
            .route("/previews/:id", get(preview::info))
            .route("/previews/:id/reset", post(preview::reset))
            .route("/previews/:id/request", post(preview::send_request))
            .route("/previews/:id/exec", post(preview::exec))
            .route("/previews/:id/enter", post(preview::enter))
            .route("/previews/:id/web", get(preview::moved))
            .route("/previews/:id/web/*path", get(preview::moved))
            .with_state(Arc::new(preview::PreviewState::new(
                state.code.clone(),
                Arc::clone(&jwt_secret),
                Arc::clone(&preview_passes),
                preview_origin.clone(),
            )))
    };

    let members_router = Router::new()
        .route(
            "/projects/:pid/members",
            get(crate::projects::members::list_members).put(crate::projects::members::add_member),
        )
        .route(
            "/projects/:pid/members/:uid",
            put(crate::projects::members::set_member_role)
                .delete(crate::projects::members::remove_member),
        )
        .with_state(Arc::clone(&state.db));

    let personal_router = Router::new()
        .route(
            "/me/artifacts",
            get(crate::personal::list_artifacts).post(crate::personal::create_personal_artifact),
        )
        .route(
            "/me/skills",
            get(crate::personal::list_skills).post(crate::personal::create_skill),
        )
        .route(
            "/me/skills/:id",
            get(crate::personal::get_skill)
                .put(crate::personal::update_skill)
                .delete(crate::personal::delete_skill),
        )
        .with_state(Arc::clone(&state.db));

    let api_key_state = Arc::new(api_keys::ApiKeyState {
        store: Arc::clone(&state.api_key_store),
    });
    let api_key_router = Router::new()
        .route(
            "/api-keys",
            get(api_keys::list_api_keys).post(api_keys::create_api_key),
        )
        .route(
            "/api-keys/:id",
            patch(api_keys::rename_api_key).delete(api_keys::revoke_api_key),
        )
        .with_state(api_key_state);

    let conv_router = Router::new()
        .route(
            "/conversations",
            get(conv_handlers::list).post(conv_handlers::create),
        )
        .route(
            "/conversations/:id",
            get(conv_handlers::get)
                .put(conv_handlers::update)
                .delete(conv_handlers::delete),
        )
        .with_state(Arc::clone(&conv_state));

    let chat_layout_router = Router::new()
        .route(
            "/chat-layouts/current",
            get(chat_layout_handlers::get_current).put(chat_layout_handlers::put_current),
        )
        .route(
            "/chat-layouts",
            get(chat_layout_handlers::list_named).post(chat_layout_handlers::create_named),
        )
        .route(
            "/chat-layouts/:id",
            get(chat_layout_handlers::get_named)
                .put(chat_layout_handlers::update_named)
                .delete(chat_layout_handlers::delete_named),
        )
        .with_state(Arc::clone(&chat_layout_state));

    let project_state = Arc::new(
        ProjectState::with_live(
            Arc::clone(&state.db),
            Arc::clone(&state.agent),
            Arc::clone(&state.agent_builder),
            Arc::clone(&state.llm),
            Arc::clone(&state.llm_configs),
            state.system_one_config.clone(),
            state.user_key_store.clone(),
            Arc::clone(&state.pricing),
            Arc::clone(&state.collocate_registry),
            Arc::clone(&state.live),
        )
        .with_admission(Arc::clone(&state.admission)),
    );

    let skill_store = Arc::new(SkillStore::new(Arc::clone(&state.db)));

    let project_router = Router::new()
        .route(
            "/projects",
            get(proj_handlers::list_projects).post(proj_handlers::create_project),
        )
        .route(
            "/projects/:pid",
            get(proj_handlers::get_project)
                .put(proj_handlers::update_project)
                .delete(proj_handlers::delete_project),
        )
        .route(
            "/projects/:pid/conversations",
            get(proj_handlers::list_conversations).post(proj_handlers::create_conversation),
        )
        .route(
            "/projects/:pid/conversations/:cid",
            get(proj_handlers::get_conversation)
                .put(proj_handlers::update_conversation)
                .delete(proj_handlers::delete_conversation),
        )
        .route(
            "/projects/:pid/conversations/:cid/confirm-action/resume",
            post(proj_handlers::resume_confirm_action),
        )
        .route("/projects/:pid/events", get(proj_handlers::project_events))
        .route("/projects/:pid/query", post(proj_handlers::project_query))
        .route(
            "/projects/:pid/query/stream",
            post(proj_handlers::project_query_stream),
        )
        .route(
            "/projects/:pid/artifacts",
            get(proj_handlers::list_artifacts).post(proj_handlers::create_artifact_route),
        )
        .route(
            "/projects/:pid/skills",
            get(proj_handlers::list_project_skills).post(proj_handlers::create_project_skill),
        )
        .route(
            "/projects/:pid/skills/:sid",
            get(proj_handlers::get_project_skill)
                .put(proj_handlers::update_project_skill)
                .delete(proj_handlers::delete_project_skill),
        )
        .route(
            "/projects/:pid/deployment",
            get(deployment_handlers::get_project_deployment),
        )
        .route(
            "/projects/:pid/deployments",
            get(deployment_handlers::list_deployments).post(deployment_handlers::create_deployment),
        )
        .route(
            "/projects/:pid/deployments/:did",
            get(deployment_handlers::get_deployment)
                .patch(deployment_handlers::update_deployment)
                .delete(deployment_handlers::delete_deployment),
        )
        .route(
            "/projects/:pid/deployments/:did/deploy",
            post(deployment_handlers::deploy_deployment),
        )
        .route(
            "/projects/:pid/deployments/:did/redeploy",
            post(deployment_handlers::redeploy_deployment),
        )
        .route(
            "/projects/:pid/deployments/:did/destroy",
            post(deployment_handlers::destroy_deployment),
        )
        .route(
            "/projects/:pid/deployments/:did/runs",
            get(deployment_handlers::list_deployment_runs),
        )
        .route(
            "/projects/:pid/deployments/:did/environment/questions",
            post(deployment_handlers::generate_environment_questions),
        )
        .route(
            "/projects/:pid/deployments/:did/design/generate",
            post(deployment_handlers::generate_design),
        )
        .route(
            "/projects/:pid/deployments/:did/design/generate/stream",
            post(deployment_handlers::generate_design_stream),
        )
        .route(
            "/projects/:pid/deployments/:did/design/decisions",
            post(deployment_handlers::generate_design_decisions),
        )
        .route(
            "/projects/:pid/deployments/:did/design/revise",
            post(deployment_handlers::revise_design),
        )
        .route(
            "/projects/:pid/deployments/:did/design/propose-change/stream",
            post(deployment_handlers::propose_design_change_stream),
        )
        .route(
            "/projects/:pid/deployments/:did/design/pdf",
            get(deployment_handlers::get_design_pdf),
        )
        .route(
            "/projects/:pid/deployments/:did/design/content",
            put(deployment_handlers::update_design_content),
        )
        .route(
            "/projects/:pid/deployments/:did/provision/generate",
            post(deployment_handlers::generate_provision),
        )
        .route(
            "/projects/:pid/deployments/:did/provision/generate/stream",
            post(deployment_handlers::generate_provision_stream),
        )
        .route(
            "/projects/:pid/deployments/:did/provision/propose-change",
            post(deployment_handlers::propose_provision_change),
        )
        .route(
            "/projects/:pid/deployments/:did/provision/propose-change/stream",
            post(deployment_handlers::propose_provision_change_stream),
        )
        .route(
            "/projects/:pid/deployments/:did/provision/apply-change",
            post(deployment_handlers::apply_provision_change),
        )
        .route(
            "/projects/:pid/deployments/:did/context-artifacts",
            post(deployment_handlers::add_context_artifact),
        )
        .route(
            "/projects/:pid/deployments/:did/context-artifacts/link",
            post(deployment_handlers::link_context_artifact),
        )
        .route(
            "/projects/:pid/deployments/:did/context-artifacts/:aid",
            delete(deployment_handlers::remove_context_artifact),
        )
        .route(
            "/projects/:pid/deployments/:did/proposals",
            get(deployment_handlers::list_proposals)
                .post(deployment_handlers::propose_artifact_change),
        )
        .route(
            "/projects/:pid/deployments/:did/proposals/:propid/approve",
            post(deployment_handlers::approve_proposal),
        )
        .route(
            "/projects/:pid/deployments/:did/proposals/:propid/discard",
            post(deployment_handlers::discard_proposal),
        )
        .route(
            "/projects/:pid/deployments/:did/execution-plan",
            get(deployment_handlers::get_execution_plan)
                .post(deployment_handlers::set_execution_plan),
        )
        .route(
            "/projects/:pid/deployments/:did/run-dag",
            post(deployment_handlers::run_dag),
        )
        .route(
            "/projects/:pid/deployments/:did/run-destroy-dag",
            post(deployment_handlers::run_destroy_dag),
        )
        .route(
            "/templates",
            get(deployment_handlers::list_templates).post(deployment_handlers::create_template),
        )
        .route(
            "/templates/upload",
            post(deployment_handlers::upload_template),
        )
        .route(
            "/templates/:tid",
            get(deployment_handlers::get_template)
                .put(deployment_handlers::update_template)
                .delete(deployment_handlers::delete_template),
        )
        .route(
            "/templates/:tid/download",
            get(deployment_handlers::download_template),
        )
        .route("/projects/:pid/cost", get(cost::project_cost))
        .route(
            "/projects/:pid/cost/by-model",
            get(cost::project_cost_by_model),
        )
        .route(
            "/projects/:pid/cost/by-user",
            get(cost::project_cost_by_user),
        )
        .route(
            "/projects/:pid/conversations/:cid/cost",
            get(cost::conversation_cost),
        )
        .route(
            "/projects/:pid/deployments/:did/cost",
            get(cost::deployment_cost),
        )
        .route(
            "/projects/:pid/deployments/:did/cost/calls",
            get(cost::deployment_cost_calls),
        )
        .with_state(project_state);

    let machine_state = Arc::new(MachineState {
        registry: Arc::clone(&state.machine_registry),
        db: Some(Arc::clone(&state.db)),
        binary_path: state.binary_path.clone(),
        server_url,
        lxd: state.lxd.clone(),
    });

    let machines_public = machines_router(Arc::clone(&machine_state));
    let machines_protected = machines_protected_router(Arc::clone(&machine_state));

    let skills_read_router = Router::new()
        .route("/skills", get(skill_handlers::list_global_skills))
        .route("/skills/:id", get(skill_handlers::get_global_skill))
        .with_state(Arc::clone(&skill_store));

    let artifact_state = Arc::new(ArtifactState {
        db: Arc::clone(&state.db),
    });
    let artifact_router = Router::new()
        .route(
            "/artifacts/:id",
            get(artifact_handlers::get_artifact)
                .put(artifact_handlers::update_artifact_route)
                .delete(artifact_handlers::delete_artifact),
        )
        .route(
            "/artifacts/:id/download",
            get(artifact_handlers::download_artifact),
        )
        .with_state(artifact_state);

    let mut protected_router = Router::new()
        .merge(me_router)
        .merge(credential_router)
        .merge(personal_router)
        .merge(members_router)
        .merge(code_router)
        .merge(api_key_router)
        .merge(conv_router)
        .merge(chat_layout_router)
        .merge(agent_router)
        .merge(graph_router)
        .merge(llm_router)
        .merge(project_router)
        .merge(machines_protected)
        .merge(skills_read_router)
        .merge(artifact_router);

    if let Some(docs_dir) = state.docs_dir {
        let docs_router = Router::new()
            .route("/docs/:repo/:version", get(docs::handle_get_index))
            .route(
                "/docs/:repo/:version/:section/*filename",
                get(docs::handle_get_page),
            )
            .with_state(docs_dir);
        protected_router = protected_router.merge(docs_router);
    }

    let protected_router = protected_router
        .layer(from_fn_with_state(
            Arc::clone(&state.db),
            crate::access::guard::project_guard,
        ))
        .layer(from_fn_with_state(
            Arc::clone(&jwt_secret),
            auth::require_auth,
        ));

    let admin_auth_routes = Router::new()
        .route("/admin/users", get(crate::admin::handlers::list_users))
        .route(
            "/admin/conversation-moves",
            get(crate::admin::handlers::list_conversation_moves)
                .post(crate::admin::handlers::move_conversations),
        )
        .route(
            "/admin/users/:id/role",
            put(crate::admin::handlers::set_user_role),
        )
        .with_state(Arc::clone(&auth_state));

    let admin_skills_routes = Router::new()
        .route("/admin/skills", post(skill_handlers::create_global_skill))
        .route(
            "/admin/skills/:id",
            put(skill_handlers::update_global_skill).delete(skill_handlers::delete_global_skill),
        )
        .with_state(Arc::clone(&skill_store));

    let admin_graph_routes = Router::new()
        .route(
            "/admin/repositories/:name",
            delete(repositories::handle_delete_repository),
        )
        .route(
            "/admin/repositories/:name/versions/:version",
            delete(repositories::handle_delete_version),
        )
        .with_state(Arc::clone(&graph_state));

    let admin_router = admin_auth_routes
        .merge(admin_skills_routes)
        .merge(admin_graph_routes)
        .layer(from_fn_with_state(
            Arc::clone(&jwt_secret),
            auth::require_admin,
        ));

    let external_router = Router::new()
        .route("/v1/chat", post(external_chat::chat))
        .route("/v1/chat/:tracking_id", get(external_chat::history))
        .with_state(external_chat_state)
        .layer(from_fn_with_state(
            Arc::clone(&state.api_key_store),
            auth::api_keys::require_api_key,
        ));

    let harvest = Router::new()
        .merge(public_router)
        .merge(external_router)
        .merge(preview_router)
        .merge(machines_public)
        .merge(protected_router)
        .merge(admin_router)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());
    match preview_site {
        Some((site, origin)) => crate::code::previews::site::by_host(harvest, site, origin),
        None => harvest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_without_thresholds_uses_agent_defaults() {
        let t: Option<Thresholds> = None;
        assert!(
            t.is_none(),
            "an unset threshold bundle must leave agent defaults alone"
        );
    }

    #[test]
    fn thresholds_round_trip_into_the_agent_builder() {
        let t = Thresholds {
            fast_path: 0.6,
            early_synthesis: 0.9,
            relevance: 0.3,
            early_synthesis_research: 0.97,
            relevance_preserve_recent: 3,
            early_synthesis_coverage: 0.85,
            early_synthesis_min_iterations_first_turn: 6,
            early_synthesis_uniform: 0.72,
            early_synthesis_capability_gate: 0.55,
            next_action_confidence: 0.65,
        };
        assert_eq!(t.early_synthesis_capability_gate, 0.55);
        assert_eq!(t.early_synthesis_uniform, 0.72);
        assert_eq!(t.next_action_confidence, 0.65);
    }
}
