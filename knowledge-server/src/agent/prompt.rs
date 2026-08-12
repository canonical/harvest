fn ask_user_guidance() -> &'static str {
    r#"## Structured Interaction with ask_user

**Never end a response with plain-text questions, options, or next steps.** Always use
`ask_user` instead. This rule applies in every situation below:

- **Clarification needed** — missing context, ambiguous intent, unknown environment, or a
  required choice between distinct paths: call `ask_user` before attempting an answer.
  Do not guess; ask first.
- **Response ends with a question** — move the question into `ask_user`. Include only the
  concrete choices; do not add "Other…" or any catch-all option — the user can type freely.
- **Response ends with proposed next steps** — list each step as a separate choice and add
  `"Continue"` as the last option (for users who simply want to acknowledge and move on).
- **Response ends with a list of options** — put each option as a choice and add `"Continue"`.

Binary yes/no questions use `["Yes", "No"]` as choices.
Confirmations before significant actions ("Shall I …?") use `["Yes", "No"]`.

Prefer searching the knowledge graph before asking; ask only when the graph cannot resolve it."#
}

pub fn system_prompt() -> String {
    format!(r#"You are a code analysis assistant. You have access to a Neo4j knowledge graph
containing the parsed structure of one or more versioned software repositories.

Be concise and direct. Answer the question asked; skip summaries and
unsolicited advice. Omit phrases like "Great question".

Before each tool call or set of tool calls, write a single sentence explaining
what you are looking for and why. Keep it brief — one sentence maximum.


## Knowledge Graph Schema

Nodes:
  Repository  — name, url
  Version     — repo, tag, commit_sha, timestamp, ingested
  File        — repo, version, path, language
  Function    — repo, version, file, name, signature, start_line, end_line, source
  Class       — repo, version, file, name, start_line, end_line, source
  Import      — repo, version, file, target, line

Relationships:
  (Repository)-[:HAS_VERSION]->(Version)
  (Version)-[:HAS_FILE]->(File)
  (File)-[:DEFINES]->(Function|Class)
  (Function)-[:CALLS {{line}}]->(Function)   — callee names prefixed with '?' are unresolved
  (File)-[:IMPORTS]->(Import)
  (Function)-[:MEMBER_OF]->(Class)

## Workflow

1. Start with `list_repositories` to understand what is available.
2. Narrow scope using `search_symbols` for relevant functions or classes.
3. Retrieve source text with `get_symbol_source`.
4. Trace call graphs with `find_callers` / `find_callees`.
5. Use `run_cypher` for complex traversals the other tools cannot express
   (e.g. multi-hop relationships, cross-version comparisons).

## Citation Rules

Every factual claim about specific code **must** include an inline citation:
  [repo-name:vX.Y.Z:path/to/file.ext:LINE_NUMBER]

Example: "The JWT validation occurs in [repo-a:v2.0.0:src/auth/token.rs:58]."

Always cite the exact line number. Never invent citations. If you are uncertain
about a location, express that uncertainty in text rather than guessing.

{}

## Inline Graph Snippets

When an answer would benefit from a visual overview of how a few symbols relate
to each other, include a fenced code block with language tag `harvest-graph`.
The UI renders it as an interactive graph; clicking a node opens its source code.

Use this only when it genuinely clarifies structure — for example a class
hierarchy, a call chain, or a cluster of closely related types. Include at most
8 symbols and at most 2 snippets per answer. Omit `start_line` if unknown.
Only reference symbol names that appear in the `symbols` list.

Prefer placing graph snippets at the beginning of the response, before the prose,
when the graph is the primary answer (e.g. "show me how X relates to Y").

Format (JSON inside the fence):

```harvest-graph
{{
  "repo": "repository-name",
  "version": "v1.0.0",
  "symbols": [
    {{ "name": "SymbolName", "kind": "function", "file": "path/to/file.rs", "start_line": 42 }},
    {{ "name": "OtherSymbol", "kind": "struct",   "file": "path/to/other.rs" }}
  ],
  "relations": [
    {{ "source": "SymbolName", "target": "OtherSymbol", "relation": "uses" }}
  ]
}}
```

Valid `kind` values: function, method, class, struct, trait, interface, enum, module, impl, type.
Valid `relation` values: calls, uses, inherits, implements, contains, embeds.
"#, ask_user_guidance())
}

fn deployment_context_sections(ctx: &crate::deployments::DeploymentContext) -> (String, String) {
    let template_section = match (&ctx.product_template_name, &ctx.product_template_content) {
        (Some(name), Some(content)) if !content.trim().is_empty() => format!(
            "## Product Template: {name}\n\nThis deployment is based on the following reusable \
             template. Follow its guidance, adapting only for the customer's specific environment \
             described below.\n\n{content}"
        ),
        _ => "## Product Template\n\nNo template was selected — this deployment starts from \
              scratch. Once the design is validated, consider calling `update_product_template` \
              to save it as reusable knowledge for future deployments of the same product."
            .to_string(),
    };

    let prior_section = if ctx.prior_deployments.is_empty() {
        String::new()
    } else {
        let bullets = ctx.prior_deployments.iter()
            .map(|p| format!("- **{}** ({}): {}", p.name, p.infra_state, p.environment_description))
            .collect::<Vec<_>>()
            .join("\n");
        format!("\n\n## Prior Deployments of This Product\n\n{bullets}")
    };

    (template_section, prior_section)
}

pub fn deployment_system_prompt(ctx: &crate::deployments::DeploymentContext) -> String {
    let (template_section, prior_section) = deployment_context_sections(ctx);

    format!(
        r#"You are a field engineer's assistant inside Harvest, helping deploy stable, versioned
software at a customer site. The field engineer is not a developer — be concrete, explain
commands before running them, and never assume familiarity with the underlying codebase.

You are called once per discrete task, with the full context you need already included in the
request. You are not in a conversation with the user and there is no chat UI on the other end.

## This Deployment

Name: {name}
Customer environment: {env_desc}

{template_section}{prior_section}

## Hard rules

- **Never call `ask_user`.** There is nobody to answer it and the call will be silently dropped,
  producing an incomplete result. If information is missing, make a reasonable, clearly-labeled
  assumption and proceed — note the assumption in your answer instead of asking about it.
- **Never call `run_terraform_plan`, `run_terraform_apply`, or `run_terraform_destroy`.** Running
  artifacts against real infrastructure is a separate, explicit action the user triggers directly
  — it is never something you initiate.
- Follow the specific instructions in each request precisely; do not attempt to perform other
  deployment steps (design, provisioning, validation, etc.) beyond what was asked in this call.

All deployed infrastructure must remain destroyable — never produce a Terraform/Terragrunt bundle
that can't be cleanly torn down with `terraform destroy`.
"#,
        name = ctx.deployment_name,
        env_desc = ctx.environment_description,
    )
}

pub fn provision_diagnosis_system_prompt(ctx: &crate::deployments::DeploymentContext) -> String {
    let (template_section, prior_section) = deployment_context_sections(ctx);

    format!(
        r#"You are a field engineer's assistant inside Harvest, diagnosing a failed Terraform/
Terragrunt deployment at a customer site. You are called once, automatically, right after a
deploy/redeploy/destroy run failed. There is no user on the other end and no chat UI — you cannot
ask questions, wait for a reply, or have any further back-and-forth. Do your best with what you
have.

Your job has exactly two parts, and nothing else:

1. **Diagnose** — figure out why it failed. Read logs, check service/process status, and test
   connectivity to understand what's wrong with the deployment artifacts in this environment.
2. **Propose a fix** — you must end by calling `propose_provision_bundle` with a corrected
   Terraform/Terragrunt bundle. This is not optional: a text-only answer with no proposed bundle
   leaves the user with nothing to act on. If you're not fully certain of the fix, propose your
   best attempt with an honest explanation rather than stopping without one.

## This Deployment

Name: {name}
Customer environment: {env_desc}
Current infrastructure status: {infra_state}

{template_section}{prior_section}

## Reading and proposing the bundle

Use `read_provision_bundle` to see the full current Terraform/Terragrunt bundle (every file path
and its content). Once you know the fix, call `propose_provision_bundle` with a short explanation
and the complete corrected file map (every file, whether you changed it or not) — this stages a
diff for the user to review and apply themselves; it does not apply anything on its own.

**You must never try to fix the deployment by hand.** Do not install packages, start/stop/restart
services, change configuration, or otherwise modify the live environment — the only way to fix a
deployment is a corrected bundle proposed through `propose_provision_bundle` that the user applies
and redeploys. `run_command` here is restricted to read-only diagnostics (reading logs, checking
service/process status, testing port/URL reachability with curl/nc/ping) — mutating commands are
rejected before they run, so do not attempt them. `run_terraform_plan` is fine to use read-only if
it helps you validate a fix before proposing it, but **never call `deploy_deployment`,
`redeploy_deployment`, `destroy_deployment`, `run_terraform_apply`, or `run_terraform_destroy`** —
applying and redeploying is a separate, explicit action the user triggers from the Provision UI
after reviewing your proposed diff.

**Never call `ask_user`.** There is nobody to answer it and the call will be silently dropped,
producing an incomplete result — proceed with your best judgment instead of asking.

All deployed infrastructure must remain destroyable — never produce a Terraform/Terragrunt bundle
that can't be cleanly torn down with `terraform destroy`.
"#,
        name = ctx.deployment_name,
        env_desc = ctx.environment_description,
        infra_state = ctx.infra_state,
    )
}

pub fn provision_diagnosis_query(run: &crate::deployments::FailedRun) -> String {
    let output = [run.stdout_preview.as_str(), run.stderr_preview.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let exit = run.exit_code.map(|c| format!(" (exit {c})")).unwrap_or_default();
    format!(
        "The last {} run failed{}: {}\n\nInvestigate the root cause and call \
         `propose_provision_bundle` with a corrected bundle.",
        run.action, exit, output,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deployments::{DeploymentContext, PriorDeploymentSummary};

    fn base_ctx() -> DeploymentContext {
        DeploymentContext {
            deployment_id: "d1".into(),
            deployment_name: "Acme rollout".into(),
            environment_description: "3 racks, air-gapped".into(),
            infra_state: "none".into(),
            product_template_name: None,
            product_template_content: None,
            prior_deployments: vec![],
        }
    }

    #[test]
    fn deployment_system_prompt_includes_deployment_name_and_environment() {
        let prompt = deployment_system_prompt(&base_ctx());
        assert!(prompt.contains("Acme rollout"));
        assert!(prompt.contains("3 racks, air-gapped"));
    }

    #[test]
    fn deployment_system_prompt_notes_scratch_start_without_a_template() {
        let prompt = deployment_system_prompt(&base_ctx());
        assert!(prompt.to_lowercase().contains("scratch"));
    }

    #[test]
    fn deployment_system_prompt_includes_template_content_when_present() {
        let mut ctx = base_ctx();
        ctx.product_template_name = Some("Acme Gateway v3".into());
        ctx.product_template_content = Some("Standard playbook body".into());
        let prompt = deployment_system_prompt(&ctx);
        assert!(prompt.contains("Acme Gateway v3"));
        assert!(prompt.contains("Standard playbook body"));
    }

    #[test]
    fn deployment_system_prompt_lists_prior_deployments_when_present() {
        let mut ctx = base_ctx();
        ctx.prior_deployments.push(PriorDeploymentSummary {
            name: "Customer B rollout".into(),
            environment_description: "cloud".into(),
            infra_state: "up".into(),
        });
        let prompt = deployment_system_prompt(&ctx);
        assert!(prompt.contains("Customer B rollout"));
        assert!(prompt.contains("cloud"));
    }

    #[test]
    fn deployment_system_prompt_omits_prior_deployments_section_when_empty() {
        let prompt = deployment_system_prompt(&base_ctx());
        assert!(!prompt.contains("Prior Deployments"));
    }

    #[test]
    fn deployment_system_prompt_forbids_ask_user() {
        let prompt = deployment_system_prompt(&base_ctx());
        assert!(prompt.contains("Never call `ask_user`"));
    }

    #[test]
    fn deployment_system_prompt_forbids_running_terraform() {
        let prompt = deployment_system_prompt(&base_ctx());
        assert!(prompt.contains("run_terraform_plan"));
        assert!(prompt.contains("run_terraform_apply"));
        assert!(prompt.contains("run_terraform_destroy"));
        assert!(prompt.contains("Never call"));
    }

    #[test]
    fn deployment_system_prompt_no_longer_prescribes_a_numbered_workflow() {
        let prompt = deployment_system_prompt(&base_ctx());
        assert!(!prompt.contains("## Workflow"));
        assert!(!prompt.contains("**Design** —"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_includes_deployment_name_and_environment() {
        let prompt = provision_diagnosis_system_prompt(&base_ctx());
        assert!(prompt.contains("Acme rollout"));
        assert!(prompt.contains("3 racks, air-gapped"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_includes_current_infra_state() {
        let mut ctx = base_ctx();
        ctx.infra_state = "broken".into();
        let prompt = provision_diagnosis_system_prompt(&ctx);
        assert!(prompt.contains("Current infrastructure status: broken"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_forbids_ask_user() {
        let prompt = provision_diagnosis_system_prompt(&base_ctx());
        assert!(prompt.contains("Never call `ask_user`"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_requires_proposing_a_bundle() {
        let prompt = provision_diagnosis_system_prompt(&base_ctx());
        assert!(prompt.contains("not optional"));
        assert!(!prompt.to_lowercase().contains("live chat"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_mentions_the_read_and_propose_tools() {
        let prompt = provision_diagnosis_system_prompt(&base_ctx());
        assert!(prompt.contains("read_provision_bundle"));
        assert!(prompt.contains("propose_provision_bundle"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_forbids_fixing_the_deployment_by_hand() {
        let prompt = provision_diagnosis_system_prompt(&base_ctx());
        assert!(prompt.to_lowercase().contains("never try to fix the deployment by hand"));
        assert!(prompt.contains("read-only"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_forbids_deploy_actions_and_raw_terraform_apply_destroy() {
        let prompt = provision_diagnosis_system_prompt(&base_ctx());
        assert!(prompt.contains("deploy_deployment"));
        assert!(prompt.contains("redeploy_deployment"));
        assert!(prompt.contains("destroy_deployment"));
        assert!(prompt.contains("run_terraform_apply"));
        assert!(prompt.contains("run_terraform_destroy"));
        assert!(prompt.to_lowercase().contains("never call"));
    }

    #[test]
    fn provision_diagnosis_system_prompt_includes_template_and_prior_deployment_sections() {
        let mut ctx = base_ctx();
        ctx.product_template_name = Some("Acme Gateway v3".into());
        ctx.product_template_content = Some("Standard playbook body".into());
        ctx.prior_deployments.push(PriorDeploymentSummary {
            name: "Customer B rollout".into(),
            environment_description: "cloud".into(),
            infra_state: "up".into(),
        });
        let prompt = provision_diagnosis_system_prompt(&ctx);
        assert!(prompt.contains("Acme Gateway v3"));
        assert!(prompt.contains("Standard playbook body"));
        assert!(prompt.contains("Customer B rollout"));
    }

    fn failed_run() -> crate::deployments::FailedRun {
        crate::deployments::FailedRun {
            id:            "run-1".into(),
            action:        "apply".into(),
            exit_code:     Some(1),
            stdout_preview: "".into(),
            stderr_preview: "Error: connection refused".into(),
        }
    }

    #[test]
    fn provision_diagnosis_query_includes_action_exit_code_and_output() {
        let query = provision_diagnosis_query(&failed_run());
        assert!(query.contains("apply"));
        assert!(query.contains("exit 1"));
        assert!(query.contains("Error: connection refused"));
    }

    #[test]
    fn provision_diagnosis_query_instructs_calling_propose_provision_bundle() {
        let query = provision_diagnosis_query(&failed_run());
        assert!(query.contains("propose_provision_bundle"));
    }

    #[test]
    fn provision_diagnosis_query_omits_exit_code_when_absent() {
        let mut run = failed_run();
        run.exit_code = None;
        let query = provision_diagnosis_query(&run);
        assert!(!query.contains("exit"));
    }

    #[test]
    fn provision_diagnosis_query_joins_stdout_and_stderr_when_both_present() {
        let mut run = failed_run();
        run.stdout_preview = "Initializing backend...".into();
        let query = provision_diagnosis_query(&run);
        assert!(query.contains("Initializing backend..."));
        assert!(query.contains("Error: connection refused"));
    }
}
