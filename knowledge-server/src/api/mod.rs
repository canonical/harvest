pub mod docs;
pub mod graph;
pub mod llm;
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

use crate::agent::{
    artifact_tools, deployment_tools, graph_tools, issue_tools, lxd_tools, machine_tools,
    port_forward_tools, prompt, skill_tools, terraform_tools, tool, Agent,
};
use crate::artifacts::handlers::{self as artifact_handlers, ArtifactState};
use crate::skills::{handlers as skill_handlers, SkillStore};
use crate::auth::{self, handlers as auth_handlers, AuthState};
use crate::config::UiConfig;
use crate::config::AuthConfig;
use crate::conversations::handlers::{self as conv_handlers, ConvState};
use crate::deployments::{self, handlers as deployment_handlers, FailedRun};
use crate::issues::handlers as issue_handlers;
use crate::llm::LlmProvider;
use crate::lxd::LxdClient;
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
    pub ui:               Arc<UiConfig>,
    pub machine_registry: Arc<MachineRegistry>,
    pub agent_builder:    Arc<ProjectAgentBuilder>,
    pub binary_path:      Option<PathBuf>,
    pub llm:              Arc<dyn LlmProvider>,
    pub lxd:              Option<Arc<LxdClient>>,
}

#[derive(Clone)]
pub struct ProjectAgentBuilder {
    pub llm:                        Arc<dyn LlmProvider>,
    pub neo4j:                      Arc<Neo4jClient>,
    pub registry:                   Arc<MachineRegistry>,
    pub skills:                     Arc<SkillStore>,
    pub lxd:                        Option<Arc<LxdClient>>,
    pub server_url:                 String,
    pub max_iterations:             usize,
    pub compaction_threshold_chars: usize,
    pub compaction_keep_last:       usize,
}

impl ProjectAgentBuilder {
    fn base_tools(&self, project_id: String) -> Vec<Box<dyn tool::Tool>> {
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
            store:      Arc::clone(&self.skills),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(skill_tools::LoadSkillTool {
            store:      Arc::clone(&self.skills),
            project_id: project_id.clone(),
        }));
        if let Some(lxd) = &self.lxd {
            tools.push(Box::new(lxd_tools::CreateLxdAgentTool {
                neo4j:      Arc::clone(&self.neo4j),
                lxd:        Arc::clone(lxd),
                server_url: self.server_url.clone(),
                project_id: project_id.clone(),
            }));
        }
        tools.push(Box::new(lxd_tools::DeleteAgentTool {
            neo4j:      Arc::clone(&self.neo4j),
            lxd:        self.lxd.clone(),
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(port_forward_tools::ListPortForwardsTool {
            neo4j:      Arc::clone(&self.neo4j),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(port_forward_tools::CreatePortForwardTool {
            neo4j:      Arc::clone(&self.neo4j),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(port_forward_tools::UpdatePortForwardTool {
            neo4j:      Arc::clone(&self.neo4j),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(port_forward_tools::DeletePortForwardTool {
            neo4j:      Arc::clone(&self.neo4j),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(artifact_tools::GenerateArtifactTool {
            neo4j:      Arc::clone(&self.neo4j),
            project_id: project_id.clone(),
            server_url: self.server_url.clone(),
        }));
        tools.push(Box::new(terraform_tools::RunTerraformPlanTool {
            neo4j:      Arc::clone(&self.neo4j),
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(terraform_tools::RunTerraformApplyTool {
            neo4j:      Arc::clone(&self.neo4j),
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(terraform_tools::RunTerraformDestroyTool {
            neo4j:      Arc::clone(&self.neo4j),
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools
    }

    pub fn build(&self, project_id: String) -> Arc<Agent> {
        let tools = self.base_tools(project_id);
        Arc::new(
            Agent::new(Arc::clone(&self.llm), tools, self.max_iterations)
                .with_compaction(self.compaction_threshold_chars, self.compaction_keep_last),
        )
    }

    pub fn build_for_deployment(
        &self,
        project_id: String,
        group_id:   String,
        ctx:        &deployments::DeploymentContext,
    ) -> Arc<Agent> {
        let mut tools = self.base_tools(project_id.clone());
        tools.push(Box::new(deployment_tools::LinkDeploymentArtifactTool {
            neo4j:         Arc::clone(&self.neo4j),
            project_id:    project_id.clone(),
            deployment_id: ctx.deployment_id.clone(),
        }));
        tools.push(Box::new(deployment_tools::UpdateProductTemplateTool {
            neo4j:         Arc::clone(&self.neo4j),
            group_id,
            deployment_id: ctx.deployment_id.clone(),
        }));
        Arc::new(
            Agent::new(Arc::clone(&self.llm), tools, self.max_iterations)
                .with_compaction(self.compaction_threshold_chars, self.compaction_keep_last)
                .with_system_prompt(prompt::deployment_system_prompt(ctx)),
        )
    }

    /// Tools available to the automatic post-failure triage agent (`build_for_issue_triage`) and
    /// the interactive per-issue chat agent (`build_for_issue_chat`): read-only diagnostics plus
    /// the ability to read the bundle and propose a fix, structurally excluding
    /// `RunCommandTool`/apply/destroy/deploy tools the same way `provision_diagnosis_tools` does.
    fn issue_tools_base(&self, project_id: String) -> Vec<Box<dyn tool::Tool>> {
        let mut tools = graph_tools::all_tools(Arc::clone(&self.neo4j));
        tools.push(Box::new(machine_tools::ListAgentsTool {
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(machine_tools::RunReadOnlyCommandTool {
            registry:   Arc::clone(&self.registry),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(skill_tools::ListSkillsTool {
            store:      Arc::clone(&self.skills),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(skill_tools::LoadSkillTool {
            store:      Arc::clone(&self.skills),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(port_forward_tools::ListPortForwardsTool {
            neo4j:      Arc::clone(&self.neo4j),
            project_id: project_id.clone(),
        }));
        tools.push(Box::new(terraform_tools::RunTerraformPlanTool {
            neo4j:      Arc::clone(&self.neo4j),
            registry:   Arc::clone(&self.registry),
            project_id,
        }));
        tools
    }

    /// One-shot autonomous run triggered automatically after any failed deploy/redeploy/destroy,
    /// unconditionally (not gated on the user having the page open). Matches or creates issues for
    /// each distinct failure and proposes a fix for new/updated ones.
    pub fn build_for_issue_triage(
        &self,
        project_id: String,
        _group_id:  String,
        ctx:        &deployments::DeploymentContext,
        run:        &FailedRun,
    ) -> Arc<Agent> {
        let mut tools = self.issue_tools_base(project_id.clone());
        tools.push(Box::new(deployment_tools::ReadProvisionBundleTool {
            neo4j:         Arc::clone(&self.neo4j),
            project_id:    project_id.clone(),
            deployment_id: ctx.deployment_id.clone(),
        }));
        tools.push(Box::new(issue_tools::ListDeploymentIssuesTool {
            neo4j:         Arc::clone(&self.neo4j),
            project_id:    project_id.clone(),
            deployment_id: ctx.deployment_id.clone(),
        }));
        tools.push(Box::new(issue_tools::CreateOrLinkIssueTool {
            neo4j:         Arc::clone(&self.neo4j),
            project_id:    project_id.clone(),
            deployment_id: ctx.deployment_id.clone(),
            run:           run.clone(),
            created_by:    "harvest".to_string(),
        }));
        Arc::new(
            Agent::new(Arc::clone(&self.llm), tools, self.max_iterations)
                .with_compaction(self.compaction_threshold_chars, self.compaction_keep_last)
                .with_system_prompt(prompt::issue_triage_system_prompt(ctx)),
        )
    }

    /// Interactive per-issue investigation chat: read-only diagnostics plus the ability to propose
    /// (never apply) a fix. `ask_user` stays enabled here, unlike triage — this is a live chat with
    /// a user on the other end, not a one-shot background job.
    pub fn build_for_issue_chat(
        &self,
        project_id: String,
        _group_id:  String,
        ctx:        &deployments::DeploymentContext,
        issue_id:   &str,
        issue_title: &str,
        issue_description: &str,
    ) -> Arc<Agent> {
        let mut tools = self.issue_tools_base(project_id.clone());
        tools.push(Box::new(deployment_tools::ReadProvisionBundleTool {
            neo4j:         Arc::clone(&self.neo4j),
            project_id:    project_id.clone(),
            deployment_id: ctx.deployment_id.clone(),
        }));
        tools.push(Box::new(issue_tools::ProposeIssueSolutionTool {
            neo4j:         Arc::clone(&self.neo4j),
            project_id:    project_id.clone(),
            deployment_id: ctx.deployment_id.clone(),
            issue_id:      issue_id.to_string(),
        }));
        Arc::new(
            Agent::new(Arc::clone(&self.llm), tools, self.max_iterations)
                .with_compaction(self.compaction_threshold_chars, self.compaction_keep_last)
                .with_system_prompt(prompt::issue_chat_system_prompt(ctx, issue_title, issue_description)),
        )
    }
}

pub async fn router(state: AppState, cache: Arc<GraphCache>, server_url: String) -> Router {
    let graph_state = Arc::new(GraphState {
        neo4j: Arc::clone(&state.neo4j),
        cache,
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
        neo4j:          Arc::clone(&state.neo4j),
        config:         Arc::clone(&state.auth),
        ui:             Arc::clone(&state.ui),
        http,
        oidc_endpoints,
        oauth_sessions: Arc::new(dashmap::DashMap::new()),
        lxd_enabled:    state.lxd.is_some(),
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
        .route("/auth/oidc",              get(auth_handlers::oidc_redirect))
        .route("/auth/oidc/callback",     get(auth_handlers::oidc_callback))
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

    let llm_state = Arc::new(llm::LlmState::new(Arc::clone(&state.llm)));
    let llm_router = Router::new()
        .route("/llm/providers", get(llm::list_providers))
        .with_state(llm_state);

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

    let skill_store = Arc::new(SkillStore::new(Arc::clone(&state.neo4j)));

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
        .route("/projects/:pid/conversations/:cid/confirm-action/resume",
               post(proj_handlers::resume_confirm_action))
        .route("/projects/:pid/events",        get(proj_handlers::project_events))
        .route("/projects/:pid/query",         post(proj_handlers::project_query))
        .route("/projects/:pid/query/stream",  post(proj_handlers::project_query_stream))
        .route("/projects/:pid/memories",
               get(proj_handlers::list_memories).post(proj_handlers::create_memory))
        .route("/projects/:pid/memories/:mid",
               get(proj_handlers::get_memory)
               .put(proj_handlers::update_memory)
               .delete(proj_handlers::delete_memory))
        .route("/projects/:pid/artifacts",
               get(proj_handlers::list_artifacts).post(proj_handlers::create_artifact_route))
        .route("/projects/:pid/skills",
               get(proj_handlers::list_project_skills).post(proj_handlers::create_project_skill))
        .route("/projects/:pid/skills/:sid",
               get(proj_handlers::get_project_skill)
               .put(proj_handlers::update_project_skill)
               .delete(proj_handlers::delete_project_skill))
        .route("/projects/:pid/tasks",
               get(proj_handlers::list_tasks).post(proj_handlers::create_task))
        .route("/projects/:pid/tasks/:tid",
               patch(proj_handlers::update_task).delete(proj_handlers::delete_task))
        .route("/projects/:pid/tasks/:tid/run",
               post(proj_handlers::run_task))
        .route("/projects/:pid/tasks/:tid/logs",
               get(proj_handlers::get_task_logs))
        .route("/projects/:pid/deployments",
               get(deployment_handlers::list_deployments).post(deployment_handlers::create_deployment))
        .route("/projects/:pid/deployments/:did",
               get(deployment_handlers::get_deployment)
               .patch(deployment_handlers::update_deployment)
               .delete(deployment_handlers::delete_deployment))
        .route("/projects/:pid/deployments/:did/deploy",   post(deployment_handlers::deploy_deployment))
        .route("/projects/:pid/deployments/:did/redeploy", post(deployment_handlers::redeploy_deployment))
        .route("/projects/:pid/deployments/:did/destroy",  post(deployment_handlers::destroy_deployment))
        .route("/projects/:pid/deployments/:did/runs",     get(deployment_handlers::list_deployment_runs))
        .route("/projects/:pid/deployments/:did/environment/questions",
               post(deployment_handlers::generate_environment_questions))
        .route("/projects/:pid/deployments/:did/design/generate",  post(deployment_handlers::generate_design))
        .route("/projects/:pid/deployments/:did/design/decisions", post(deployment_handlers::generate_design_decisions))
        .route("/projects/:pid/deployments/:did/design/revise",    post(deployment_handlers::revise_design))
        .route("/projects/:pid/deployments/:did/provision/generate", post(deployment_handlers::generate_provision))
        .route("/projects/:pid/deployments/:did/provision/propose-change", post(deployment_handlers::propose_provision_change))
        .route("/projects/:pid/deployments/:did/provision/apply-change", post(deployment_handlers::apply_provision_change))
        .route("/projects/:pid/issues", get(issue_handlers::list_issues))
        .route("/projects/:pid/issues/:iid", get(issue_handlers::get_issue))
        .route("/projects/:pid/issues/:iid/status", patch(issue_handlers::update_issue_status_route))
        .route("/projects/:pid/issues/:iid/comments", post(issue_handlers::create_issue_comment))
        .route("/projects/:pid/issues/:iid/apply-solution", post(issue_handlers::apply_issue_solution))
        .route("/projects/:pid/issues/:iid/redeploy", post(issue_handlers::redeploy_from_issue))
        .route("/projects/:pid/issues/:iid/chat", post(issue_handlers::issue_chat))
        .route("/groups/:gid/templates",
               get(deployment_handlers::list_templates).post(deployment_handlers::create_template))
        .route("/groups/:gid/templates/:tid",
               get(deployment_handlers::get_template)
               .put(deployment_handlers::update_template)
               .delete(deployment_handlers::delete_template))
        .with_state(project_state);

    let machine_state = Arc::new(MachineState {
        registry:    Arc::clone(&state.machine_registry),
        neo4j:       Some(Arc::clone(&state.neo4j)),
        binary_path: state.binary_path.clone(),
        server_url,
        lxd:         state.lxd.clone(),
    });

    let machines_public = machines_router(Arc::clone(&machine_state));
    let machines_protected = machines_protected_router(Arc::clone(&machine_state));

    let skills_read_router = Router::new()
        .route("/skills",     get(skill_handlers::list_global_skills))
        .route("/skills/:id", get(skill_handlers::get_global_skill))
        .with_state(Arc::clone(&skill_store));

    let artifact_state = Arc::new(ArtifactState { neo4j: Arc::clone(&state.neo4j) });
    let artifact_router = Router::new()
        .route("/artifacts/:id",
               get(artifact_handlers::get_artifact)
               .put(artifact_handlers::update_artifact_route)
               .delete(artifact_handlers::delete_artifact))
        .route("/artifacts/:id/download", get(artifact_handlers::download_artifact))
        .with_state(artifact_state);

    let mut protected_router = Router::new()
        .merge(me_router)
        .merge(conv_router)
        .merge(agent_router)
        .merge(graph_router)
        .merge(llm_router)
        .merge(project_router)
        .merge(machines_protected)
        .merge(skills_read_router)
        .merge(artifact_router);

    if let Some(docs_dir) = state.docs_dir {
        let docs_router = Router::new()
            .route("/docs/:repo/:version",              get(docs::handle_get_index))
            .route("/docs/:repo/:version/:section/*filename", get(docs::handle_get_page))
            .with_state(docs_dir);
        protected_router = protected_router.merge(docs_router);
    }

    let protected_router = protected_router
        .layer(from_fn_with_state(Arc::clone(&jwt_secret), auth::require_auth));

    let admin_auth_routes = Router::new()
        .route("/admin/users",          get(crate::admin::handlers::list_users))
        .route("/admin/users/:id/role", put(crate::admin::handlers::set_user_role))
        .route("/admin/users/:id/groups", put(crate::admin::handlers::set_user_groups))
        .route("/admin/groups",         get(crate::admin::handlers::list_groups)
                                       .post(crate::admin::handlers::create_group))
        .route("/admin/groups/:id",     delete(crate::admin::handlers::delete_group))
        .route("/admin/groups/:id/default", put(crate::admin::handlers::set_group_default))
        .with_state(Arc::clone(&auth_state));

    let admin_skills_routes = Router::new()
        .route("/admin/skills",     post(skill_handlers::create_global_skill))
        .route("/admin/skills/:id", put(skill_handlers::update_global_skill)
                                   .delete(skill_handlers::delete_global_skill))
        .with_state(Arc::clone(&skill_store));

    let admin_router = admin_auth_routes
        .merge(admin_skills_routes)
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
