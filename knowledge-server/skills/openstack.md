---
name: openstack
description: Deploy and operate Canonical OpenStack (Sunbeam) clusters, and manage cloud resources with the openstack CLI — bootstrap, scaling, upgrades, maintenance, networking, and troubleshooting
---

# Canonical OpenStack (Sunbeam)

Canonical OpenStack is Canonical's distribution of upstream OpenStack, built on **Sunbeam** — a `snap install openstack` that gives you the `sunbeam` CLI for lifecycle management (bootstrap, scale, upgrade, maintenance) plus the standard `openstack` CLI (python-openstackclient) for day-2 cloud operations. Under the hood, Sunbeam deploys OpenStack services as Kubernetes charms via Juju, backed by Canonical Kubernetes (`k8s`), MicroCeph for storage, and OVN for networking.

## Core concepts

- **Node roles**: `control`, `compute`, `storage` — a node can hold one or more roles
- **Primary node**: the first node bootstrapped; hosts the Juju controller — removing it destroys the whole deployment
- **Cluster**: the set of nodes managed by `sunbeam cluster` (backed by `snap.openstack.clusterd`)
- **Juju underneath**: OpenStack services run as charms in the `openstack` model; `admin/controller` is the Juju controller model
- **Manifest**: YAML file describing channels/config for a deployment, used with `--manifest` on bootstrap/refresh
- **Project/domain/user**: standard Keystone scoping — most `openstack` CLI commands are scoped to a project via your sourced credentials

Underlying services: Keystone, Nova, Neutron, Cinder (+ Cinder-Ceph), Glance, Heat, Horizon, Placement, Octavia, Designate, Magnum, Barbican, Aodh, Ceilometer, Gnocchi, RabbitMQ, MySQL, OVN, Vault, Traefik.

## Install and bootstrap

```bash
sudo snap install openstack

# prepare the node (adds required groups, packages) — run as a normal user, not root
sunbeam prepare-node-script --bootstrap | bash -x && newgrp snap_daemon

# bootstrap the first (primary) node
sunbeam cluster bootstrap --accept-defaults --role control,compute,storage
sunbeam cluster bootstrap --manifest ./manifest.yaml   # pin channels/config

# configure the cloud: creates demo project, network, flavors; writes credentials
sunbeam configure --accept-defaults --openrc demo-openrc

# smoke-test with a throwaway VM
sunbeam launch ubuntu --name test
```

## Cluster membership (scale out / in)

Nodes are identified by **FQDN only** — short hostnames are not supported. Every node must run the same OpenStack snap revision/track before joining.

```bash
# on the new node
sunbeam prepare-node-script | bash -x && newgrp snap_daemon
sudo snap install openstack     # match the primary's channel exactly

# on the primary node: mint a one-time join token
sunbeam cluster add <new-node-fqdn> --output token.txt

# on the new node: consume the token and pick roles
cat token.txt | sunbeam cluster join --role control,compute -

# on the primary: re-run resize after adding/removing control nodes
sunbeam cluster resize

sunbeam cluster list       # verify membership and roles
```

MAAS-managed deployments use `sunbeam cluster deploy` on the primary instead of the manual add/join flow (machine must already be enlisted, commissioned, and tagged in MAAS).

Scaling in is the reverse: drain/maintenance the node first (see below), then `sunbeam cluster remove <fqdn>`.

## Maintenance mode

Use before patching, rebooting, or otherwise touching a node so instances aren't disrupted underneath you.

```bash
sunbeam enable maintenance                       # one-time feature activation

sunbeam cluster maintenance enable <node> --dry-run   # preview the plan first
sunbeam cluster maintenance enable <node>

# control how running instances are handled while the node is down
sunbeam cluster maintenance enable <node> --disable-migration=live   # cold-migrate everything
sunbeam cluster maintenance enable <node> --disable-migration=cold   # only live-migrate active instances
sunbeam cluster maintenance enable <node> --disable-migration        # stop active instances, ignore inactive

sunbeam cluster maintenance disable <node> --dry-run
sunbeam cluster maintenance disable <node>
```

Both enable/disable prompt for `[y/n]` confirmation after showing the plan — read it before confirming, it tells you what will move where.

## Upgrades

Refreshes must be run in this order — doing it out of order (e.g. refreshing everything before Vault) risks a sealed/inconsistent control plane:

```bash
sudo snap refresh openstack          # on every node first, so the snap itself is current

sunbeam cluster refresh k8s          # 1. Kubernetes substrate (patch versions only)
sunbeam cluster refresh vault        # 2. Vault
sunbeam cluster refresh mysql        # 3. MySQL (temporarily scales to an odd unit count)
sunbeam cluster refresh              # 4. everything else

sunbeam cluster refresh --manifest ./manifest.yaml
sunbeam cluster refresh --force
sunbeam cluster refresh mysql --reset-mysql-upgrade-state   # if a mysql refresh was interrupted
```

Things to think about:
- **Cross-track refreshes are not supported** — you cannot go from `2024.1/stable` to `2025.1/stable` with `refresh`; that needs a fresh deployment/migration path.
- Kubernetes `refresh k8s` only handles patch bumps (e.g. `1.32.1` → `1.32.4`), not minor/major version jumps.
- After refreshing, Vault comes back **sealed** and needs manual unsealing unless dev mode is enabled.

## Day-2 `openstack` CLI usage

```bash
# admin credentials (full cloud visibility)
sunbeam openrc > admin-openrc
source admin-openrc

# or the project-scoped creds written by `sunbeam configure`
source demo-openrc

openstack server list
openstack server create --flavor m1.small --image ubuntu --network demo-network --key-name mykey myvm
openstack server delete myvm

openstack network list
openstack network create mynet
openstack subnet create --network mynet --subnet-range 10.0.0.0/24 mysubnet
openstack router create myrouter
openstack router set myrouter --external-gateway <external-net>
openstack router add subnet myrouter mysubnet

openstack security group create web
openstack security group rule create --protocol tcp --dst-port 22 web
openstack keypair create --public-key ~/.ssh/id_ed25519.pub mykey

openstack volume create --size 10 myvol
openstack server add volume myvm myvol

openstack image list
openstack project list
openstack project create --domain default myproject
```

Never run `openstack`/`sunbeam` as root — source credentials and run as the local user that has `snap_daemon` group membership (from `prepare-node-script`).

## Live migration

Requires the **admin** role. Block migration is mandatory for instances with local (non-shared) storage and can noticeably load the network; memory-heavy instances may need to pause briefly to complete.

```bash
openstack hypervisor list
openstack server show <server-id> -c flavor
openstack flavor show <flavor> -c vcpus -c ram -c disk   # check destination has capacity first
openstack host show <destination-host>

openstack server migrate --live-migration <server-id>
openstack server migrate --live-migration --os-compute-api-version 2.30 --host <hypervisor-name> <server-id>
openstack server migrate --block-migration --live-migration <server-id>   # local-storage instances
```

## Backup and restore

Backups cover the **control plane only** — MySQL, Vault, Juju, and Kubernetes infra state. Workload/instance data is not included; back that up separately (volume snapshots, in-guest backups).

```bash
# MySQL (run against a non-leader unit)
juju run mysql/0 create-backup --wait 1m
juju scale-application mysql 1
juju run mysql/leader restore-backup backup-id=<backup-id>
juju scale-application mysql 3

# Vault
juju run vault/leader create-backup
juju run vault/leader restore-backup backup-id=<backup-id>

# Kubernetes control-plane state (Velero)
juju run velero-operator/0 create-backup target=infra-backup-operator:cluster-infra-backup
juju run velero-operator/0 create-backup target=infra-backup-operator:namespaced-infra-backup

# Juju itself
juju export-bundle --model=openstack --filename=openstack-bundle.yaml
juju create-backup --model=${CONTROLLERS_MODEL} --filename=juju-ctrl-backup.tar.gz
tar -czf juju-credentials.tar.gz ~/.local/share/juju/*

# sunbeam-clusterd state
juju exec -a sunbeam-clusterd -- tar -cvf /home/ubuntu/backup.tar /var/snap/openstack/common/state/database
```

## Removing the primary node

**Removing the primary node destroys the entire deployment.** Remove every non-primary node first (`sunbeam cluster remove`); only tear down the primary once nothing else depends on it. Applies to manual bare-metal deployments only.

```bash
juju destroy-model --destroy-storage --no-prompt --force --no-wait openstack
juju destroy-model --destroy-storage --no-prompt --force --no-wait admin/openstack-machines
juju destroy-controller --no-prompt --destroy-storage --force --no-wait localhost-localhost

sudo /sbin/remove-juju-services
sudo snap remove --purge juju
sudo snap remove --purge openstack-hypervisor
sudo snap remove --purge openstack
sudo snap remove --purge k8s
sudo snap remove --purge microovn
sudo snap remove --purge microceph

rm -rf ~/.local/share/juju ~/.local/share/openstack
sudo rm -rf /var/lib/juju/dqlite
```

## Debugging and troubleshooting

```bash
# cluster and Juju state
sunbeam cluster list
juju status -m admin/controller
juju status -m openstack
sunbeam utils juju-login              # re-auth to the Juju dashboard/CLI

# Kubernetes substrate
sudo k8s status
sudo k8s inspect
sudo k8s kubectl get pods --namespace openstack
sudo k8s kubectl describe --namespace openstack pod <pod_name>
sudo k8s kubectl logs --namespace openstack --container charm <pod_name>

# hypervisor (compute node)
sudo systemctl status snap.openstack-hypervisor.*
sudo journalctl -xe -u snap.openstack-hypervisor.*

# storage
sudo microceph status
sudo ceph -s

# clusterd itself
sudo systemctl status snap.openstack.clusterd.service
sudo journalctl -xe -u snap.openstack.clusterd.service

# terraform plans backing sunbeam operations
sunbeam plans list
sunbeam plans unlock <plan_name>       # if a plan is stuck locked after a failed run
```

### Network debugging (OVN)

```bash
juju ssh -m openstack --container ovn-nb-db-server ovn-central/0
juju ssh -m openstack --container ovn-sb-db-server ovn-central/0

ovn-nbctl show
ovn-sbctl show
ovn-sbctl lflow-list

sudo tcpdump -i br-ex "icmp[0] == 8" -w ping.pcap
sudo openstack-hypervisor.ovs-appctl ofproto/trace br-ex icmp,in_port=$PORT,dl_src=$MAC,dl_dst=$DEST_MAC,nw_src=$SRC_IP,nw_dst=$DEST_IP,nw_ttl=64,icmp_type=8,icmp_code=0
sudo openstack-hypervisor.ovs-vsctl find interface ofport=2 | grep -E "^name"
```

## Common patterns and things to think about

- Nodes must be addressed by FQDN everywhere — short hostnames silently fail to join/cluster correctly
- Match the OpenStack snap channel across all nodes before joining or refreshing; mismatched versions are a common source of cluster-join failures
- Always `--dry-run` a maintenance-mode transition first and read the migration plan before confirming
- Refresh order matters: k8s → vault → mysql → everything else; never skip straight to the full refresh
- Vault needs manual unsealing after a refresh unless it's running in dev mode
- Backups don't cover workload data — plan volume/instance backups separately from control-plane backups
- Block-migrate only when necessary (local storage); it's heavier on the network than shared-storage live migration
- Deployments on snap-openstack revisions before 998 can see up to 5 minutes of intermittent API failures when a control node goes down (LP #2150551) — no fix except upgrading past that revision
- Treat removing the primary node as destructive and irreversible; double-check all other nodes are already removed
