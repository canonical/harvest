fn ask_user_guidance() -> &'static str {
    r#"## Structured Interaction with ask_user

**Never end a response with plain-text questions, options, or next steps.** Always use
`ask_user` instead. This rule applies in every situation below:

- **Clarification needed** — missing context, ambiguous intent, unknown environment, or a
  required choice between distinct paths: call `ask_user` before attempting an answer.
  Do not guess; ask first.
- **Never narrate which tool you are about to call.** Do not write "I will use the
  ask_user tool to…" or "Let me ask you…". Simply call the tool — the UI presents the
  question automatically.
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

Before each tool call or set of tool calls, write a single sentence about your
intent — what you are looking for or trying to accomplish. Never mention tool
names, API names, or internal mechanisms. The user should understand your goal,
not your method.

When reasoning through a problem, use structured formats: numbered or bulleted
lists for sequences of items.

## Mermaid diagrams

Include Mermaid diagrams in your responses as often as possible. The UI renders
fenced ```mermaid code blocks as visual diagrams.

- **Prefer simple, focused diagrams.** A small diagram that clarifies one idea
  beats a large one that tries to cover everything. Prefer multiple small
  diagrams over one big diagram.
- **Illustrate prose with a diagram.** Whenever a paragraph describes a process,
  structure, relationships, data flow, state transitions, or any sequence of
  steps, accompany it with a pertinent Mermaid diagram that represents the
  content of that paragraph. Do not replace the prose — the diagram sits
  alongside it.
- **Match the diagram type to the content.** Use flowcharts for call chains and
  decision logic, sequence diagrams for request/message flows, class diagrams
  for type hierarchies, state diagrams for state transitions, ER diagrams for
  data models, mindmaps for taxonomy.
- **Keep it readable.** At most ~8 nodes per diagram; if a topic needs more,
  split it into several diagrams.

## Context Reuse

Before calling any tool, check whether the conversation history already contains
the answer. If the user is asking a clarifying question about something you just
discussed, or a follow-up that can be answered from prior tool results in this
conversation, answer directly from context. Tool calls are for discovering new
information, not repeating work you already did.


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

pub fn intent_classifier_prompt(latest_query: &str, history_snippet: &str) -> String {
    format!(r#"Classify the user's latest message into one of these intents.
Reply with a single word only.

- conversational: a follow-up, clarification, or greeting that can be answered
  from conversation context without new tool calls (e.g. "what does that mean?",
  "thanks", "summarize that").
- research: a question about code or architecture that requires searching the
  knowledge graph (e.g. "how does retry work?", "find all callers of X").
- action: a request to execute or change something on a machine (e.g.
  "restart nginx on build-box", "deploy the service", "create an agent").
- hybrid: a request that requires both research and action (e.g. "find the
  config file and update the timeout on the agent").

Latest user message: {latest_query}
Recent conversation context: {history_snippet}

    Intent:"#)
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

pub fn issue_triage_system_prompt(ctx: &crate::deployments::DeploymentContext) -> String {
    let (template_section, prior_section) = deployment_context_sections(ctx);

    format!(
        r#"You are a field engineer's assistant inside Harvest, triaging a failed Terraform/
Terragrunt deployment run at a customer site. You are called once, automatically, right after a
deploy/redeploy/destroy run failed. There is no user on the other end and no chat UI — you cannot
ask questions, wait for a reply, or have any further back-and-forth. Do your best with what you
have.

Your job has three parts, and nothing else:

1. **List existing issues** — call `list_deployment_issues` to see what's already tracked for this
   deployment.
2. **Match or create** — for each distinct failure in this run's output, decide whether it's the
   same root cause as an existing issue. Call `create_or_link_issue` once per distinct failure:
   `action: "link_existing"` if it matches, `action: "create"` if it's new. A single run can
   contain more than one distinct failure — call the tool once for each.
3. **Propose a fix** — for every issue you just created, and any existing issue whose fix needs
   updating, call `read_provision_bundle` then `propose_issue_solution` with a corrected
   Terraform/Terragrunt bundle for that specific issue. This is not optional for newly created
   issues: a matched-but-unfixed issue with no proposed solution leaves the user with nothing to
   act on.

## This Deployment

Name: {name}
Customer environment: {env_desc}
Current infrastructure status: {infra_state}

{template_section}{prior_section}

## Reading and proposing a fix

Use `read_provision_bundle` to see the full current Terraform/Terragrunt bundle (every file path
and its content). Once you know the fix for a given issue, call `propose_issue_solution` with that
issue's id, a short explanation, and the complete corrected file map (every file, whether you
changed it or not) — this stages a diff for the user to review and apply themselves; it does not
apply anything on its own.

**You must never try to fix the deployment by hand.** Do not install packages, start/stop/restart
services, change configuration, or otherwise modify the live environment — the only way to fix a
deployment is a corrected bundle proposed through `propose_issue_solution` that the user applies
and redeploys. `run_command` here is restricted to read-only diagnostics (reading logs, checking
service/process status, testing port/URL reachability with curl/nc/ping) — mutating commands are
rejected before they run, so do not attempt them. `run_terraform_plan` is fine to use read-only if
it helps you validate a fix before proposing it, but **never call `deploy_deployment`,
`redeploy_deployment`, `destroy_deployment`, `run_terraform_apply`, or `run_terraform_destroy`** —
applying and redeploying is a separate, explicit action the user triggers after reviewing your
proposed diff.

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

pub fn issue_triage_query(run: &crate::deployments::FailedRun) -> String {
    let output = [run.stdout_preview.as_str(), run.stderr_preview.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let exit = run.exit_code.map(|c| format!(" (exit {c})")).unwrap_or_default();
    format!(
        "The last {} run (id `{}`) failed{}: {}\n\nCall `list_deployment_issues`, decide whether \
         this matches an existing issue (change request) or is new, call `create_or_link_issue` \
         accordingly, and propose a fix with `propose_issue_solution` for any newly created issue \
         (change request).",
        run.action, run.id, exit, output,
    )
}

pub fn issue_chat_system_prompt(
    ctx:               &crate::deployments::DeploymentContext,
    issue_title:       &str,
    issue_description: &str,
) -> String {
    let (template_section, prior_section) = deployment_context_sections(ctx);

    format!(
        r#"You are a field engineer's assistant inside Harvest, helping investigate and fix a
tracked deployment issue at a customer site. You are in a live chat with the field engineer — ask
`ask_user` when you need a decision only they can make.

## This Issue

Title: {issue_title}
Description: {issue_description}

## This Deployment

Name: {name}
Customer environment: {env_desc}
Current infrastructure status: {infra_state}

{template_section}{prior_section}

## Hard rules

- **You cannot modify anything.** `run_command` here is restricted to read-only diagnostics
  (reading logs, checking service/process status, testing port/URL reachability with
  curl/nc/ping) — mutating commands are rejected before they run. You cannot install packages,
  start/stop/restart services, change configuration, or write files.
- **The only way to fix this issue is to propose a bundle.** Use `read_provision_bundle` to see
  the current Terraform/Terragrunt bundle, and once you know the fix, call
  `propose_issue_solution` with a short explanation and the complete corrected file map (every
  file, whether changed or not). This stages a diff for the user to review; it does not apply
  anything on its own, and you never apply or redeploy — that is a separate action the user
  triggers themselves after approving your proposal.
- Never call `deploy_deployment`, `redeploy_deployment`, `destroy_deployment`,
  `run_terraform_apply`, or `run_terraform_destroy`.

All deployed infrastructure must remain destroyable — never produce a Terraform/Terragrunt bundle
that can't be cleanly torn down with `terraform destroy`.
"#,
        name = ctx.deployment_name,
        env_desc = ctx.environment_description,
        infra_state = ctx.infra_state,
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
    fn system_prompt_contains_context_reuse_rule() {
        let prompt = system_prompt();
        assert!(prompt.contains("check whether the conversation history already contains"));
        assert!(prompt.contains("answer directly from context"));
    }

    #[test]
    fn system_prompt_contains_anti_narration_rule() {
        let prompt = system_prompt();
        assert!(prompt.contains("Never mention tool"));
        assert!(prompt.contains("Never narrate which tool you are about to call"));
    }

    #[test]
    fn system_prompt_pre_tool_call_rule_does_not_mention_why() {
        let prompt = system_prompt();
        assert!(prompt.contains("what you are looking for or trying to accomplish"));
        assert!(!prompt.contains("explaining what you are looking for and why"));
    }

    #[test]
    fn intent_classifier_prompt_contains_query_and_options() {
        let prompt = intent_classifier_prompt("how does retry work?", "[user]: how?\n[assistant]: ...");
        assert!(prompt.contains("how does retry work?"));
        assert!(prompt.contains("conversational"));
        assert!(prompt.contains("research"));
        assert!(prompt.contains("action"));
        assert!(prompt.contains("hybrid"));
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
    fn issue_triage_system_prompt_includes_deployment_name_and_environment() {
        let prompt = issue_triage_system_prompt(&base_ctx());
        assert!(prompt.contains("Acme rollout"));
        assert!(prompt.contains("3 racks, air-gapped"));
    }

    #[test]
    fn issue_triage_system_prompt_forbids_ask_user() {
        let prompt = issue_triage_system_prompt(&base_ctx());
        assert!(prompt.contains("Never call `ask_user`"));
    }

    #[test]
    fn issue_triage_system_prompt_mentions_the_three_tools() {
        let prompt = issue_triage_system_prompt(&base_ctx());
        assert!(prompt.contains("list_deployment_issues"));
        assert!(prompt.contains("create_or_link_issue"));
        assert!(prompt.contains("propose_issue_solution"));
    }

    #[test]
    fn issue_triage_system_prompt_forbids_fixing_by_hand_and_deploy_actions() {
        let prompt = issue_triage_system_prompt(&base_ctx());
        assert!(prompt.to_lowercase().contains("never try to fix the deployment by hand"));
        assert!(prompt.contains("deploy_deployment"));
        assert!(prompt.contains("redeploy_deployment"));
        assert!(prompt.contains("destroy_deployment"));
        assert!(prompt.contains("run_terraform_apply"));
        assert!(prompt.contains("run_terraform_destroy"));
    }

    #[test]
    fn issue_triage_query_includes_run_id_action_exit_code_and_output() {
        let query = issue_triage_query(&failed_run());
        assert!(query.contains("run-1"));
        assert!(query.contains("apply"));
        assert!(query.contains("exit 1"));
        assert!(query.contains("Error: connection refused"));
    }

    #[test]
    fn issue_triage_query_instructs_listing_and_matching_before_proposing() {
        let query = issue_triage_query(&failed_run());
        assert!(query.contains("list_deployment_issues"));
        assert!(query.contains("create_or_link_issue"));
        assert!(query.contains("propose_issue_solution"));
    }

    #[test]
    fn issue_chat_system_prompt_includes_issue_title_and_description() {
        let prompt = issue_chat_system_prompt(&base_ctx(), "Apply fails on security group", "conflicting CIDR ranges");
        assert!(prompt.contains("Apply fails on security group"));
        assert!(prompt.contains("conflicting CIDR ranges"));
    }

    #[test]
    fn issue_chat_system_prompt_includes_deployment_name_and_environment() {
        let prompt = issue_chat_system_prompt(&base_ctx(), "t", "d");
        assert!(prompt.contains("Acme rollout"));
        assert!(prompt.contains("3 racks, air-gapped"));
    }

    #[test]
    fn issue_chat_system_prompt_allows_ask_user_unlike_triage() {
        let prompt = issue_chat_system_prompt(&base_ctx(), "t", "d");
        assert!(!prompt.contains("Never call `ask_user`"));
    }

    #[test]
    fn issue_chat_system_prompt_forbids_modifying_and_deploying() {
        let prompt = issue_chat_system_prompt(&base_ctx(), "t", "d");
        assert!(prompt.contains("cannot modify anything"));
        assert!(prompt.contains("deploy_deployment"));
        assert!(prompt.contains("redeploy_deployment"));
        assert!(prompt.contains("run_terraform_apply"));
        assert!(prompt.contains("run_terraform_destroy"));
    }

    #[test]
    fn issue_chat_system_prompt_mentions_propose_issue_solution() {
        let prompt = issue_chat_system_prompt(&base_ctx(), "t", "d");
        assert!(prompt.contains("propose_issue_solution"));
    }
}
