# Deploy mode

Deploy mode turns requirements into running infrastructure in reviewable steps, each producing an artifact a person can read and change before the next step uses it.

```text
requirements + product template ─▶ design document ─▶ deployment artifacts + plan ─▶ runs on agents
```

| Step | Input | Output | Reviewed by |
|---|---|---|---|
| Design | template's `design.md`, context artifacts, chat answers | design document | reading, editing, proposals |
| Provision | design, template artifacts and skills | Terraform, Terragrunt and Bash artifacts; an execution plan with dependencies | reading, editing, proposals |
| Deploy | plan, connected agents | runs with output | run history |

## Why templates

Most organisations deploy a few kinds of systems many times. A product template captures what is constant about one kind: the design's structure, the skills that know the products involved, and starting artifacts. The agent fills in what is specific to each deployment.

## Why proposals

Asking the agent to change a design or an artifact produces a proposal, not an edit. You see what would change and approve or discard it. Generated infrastructure code is never applied without a person seeing it.

## Projects

A Deploy project is the unit of sharing: its chats, artifacts, skills, templates, designs, deployments and agents belong to the project, and members have roles. See [permissions](../reference/permissions.md).
