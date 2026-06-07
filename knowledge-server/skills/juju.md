---
name: juju
description: Deploy and manage applications with Juju operators — models, machines, units, relations, bundles, actions, and debugging
---

# Juju

Juju is a model-driven operator framework. Operators (charms) encode application lifecycle logic. Juju manages them across clouds, MAAS, LXD, and Kubernetes.

## Core concepts

- **Controller**: manages one or more models; hosts the Juju state database
- **Model**: a deployment environment (collection of applications + machines)
- **Application**: a charm deployed zero or more times as units
- **Unit**: a single instance of an application running on a machine or pod
- **Relation**: a typed connection between applications (e.g. `db:pgsql`)
- **Bundle**: a YAML file deploying multiple applications and their relations at once

## Bootstrap and models

```bash
juju bootstrap localhost lxd-controller   # bootstrap on LXD
juju bootstrap maas-cloud maas-controller

juju add-model staging
juju switch mycontroller:staging
juju models                               # list all models
juju status                               # overview of current model
```

## Deploying applications

```bash
juju deploy postgresql
juju deploy nginx --num-units 3
juju deploy ./path/to/charm              # local charm
juju deploy cs:mysql-5                   # Charmhub with channel
juju deploy mysql --channel 8.0/stable
juju deploy bundle.yaml                  # from bundle file
```

## Scaling and configuration

```bash
juju add-unit nginx -n 2
juju remove-unit nginx/2
juju config postgresql max_connections=200
juju config postgresql                   # show current config
```

## Relations

```bash
juju integrate wordpress mysql
juju integrate wordpress:db mysql:db        # explicit endpoints
juju remove-integration wordpress mysql
juju show-unit wordpress/0 | grep relations
```

## Actions

```bash
juju actions postgresql
juju run postgresql/0 backup
juju run postgresql/leader backup --background
juju show-operation <id>
```

## Machines and SSH

```bash
juju machines
juju add-machine
juju add-machine ssh:user@10.0.0.5
juju ssh wordpress/0
juju ssh 2                               # by machine number
juju scp myfile.txt wordpress/0:/tmp/
```

## Secrets (Juju 3.x)

```bash
juju add-secret my-secret --label "db-password" key=value
juju list-secrets
juju grant-secret my-secret postgresql
juju update-secret my-secret key=newvalue
```

## Upgrades

```bash
juju upgrade-charm postgresql
juju upgrade-charm postgresql --revision 42
juju refresh postgresql --channel 14/stable
```

## Debugging

```bash
juju status --color
juju debug-log                           # streaming log tail
juju debug-log --include unit-mysql/0
juju debug-log --level ERROR
juju debug-hooks mysql/0                 # interactive hook debugger
juju show-status-log mysql/0
juju exec --unit mysql/0 -- systemctl status mysql
```

## Removing resources

```bash
juju remove-application nginx
juju remove-application nginx --destroy-storage
juju remove-machine 3 --force
juju destroy-model staging
juju destroy-controller lxd-controller --destroy-all-models
```

## Common patterns

- Check `juju status` before and after every operation
- Use `juju debug-log --include unit-<app>/<n>` to follow a specific unit's hooks
- Hooks run in `/var/lib/juju/agents/unit-<app>-<n>/charm/hooks/`
- Use `juju exec --unit app/0 -- <cmd>` to run ad-hoc commands without SSH
- Relations fire `relation-changed` hooks; check `relation-get` output when debugging
