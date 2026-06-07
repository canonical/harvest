---
name: lxd
description: Manage LXD containers and VMs — instances, images, profiles, networks, storage pools, clustering, and snapshots
---

# LXD

LXD is a system container and VM hypervisor. Containers share the host kernel; VMs use QEMU. Both are managed through the same API/CLI.

## Core concepts

- **Instance**: a container (`type: container`) or VM (`type: virtual-machine`)
- **Image**: read-only rootfs used to create instances; pulled from remotes or imported locally
- **Profile**: named set of config + devices applied to instances
- **Project**: namespace for instances, images, profiles, and networks
- **Remote**: a registry or another LXD server (`images:`, `ubuntu:`, etc.)

## Instance lifecycle

```bash
lxc launch ubuntu:22.04 mycontainer
lxc launch ubuntu:22.04 myvm --vm
lxc launch ubuntu:22.04 myvm --vm --config limits.cpu=4 --config limits.memory=8GB

lxc list
lxc info mycontainer

lxc start mycontainer
lxc stop mycontainer
lxc stop mycontainer --force
lxc restart mycontainer
lxc delete mycontainer
lxc delete mycontainer --force   # force-delete running instance
```

## Shell access and file ops

```bash
lxc exec mycontainer -- bash
lxc exec mycontainer -- /bin/sh
lxc exec mycontainer --env MY_VAR=value -- env

lxc file push localfile mycontainer/tmp/dest
lxc file pull mycontainer/etc/hosts ./hosts
lxc file edit mycontainer/etc/hosts
```

## Configuration

```bash
lxc config show mycontainer
lxc config set mycontainer limits.cpu 2
lxc config set mycontainer limits.memory 1GB
lxc config set mycontainer security.nesting true      # nested containers
lxc config set mycontainer security.privileged true   # privileged container (avoid if possible)

lxc config device add mycontainer disk1 disk source=/data path=/mnt/data
lxc config device add mycontainer eth1 nic nictype=bridged parent=br0
lxc config device remove mycontainer disk1
```

## Profiles

```bash
lxc profile list
lxc profile show default
lxc profile create production
lxc profile edit production       # opens $EDITOR
lxc profile assign mycontainer production
lxc profile add mycontainer extra-profile
```

## Images

```bash
lxc image list
lxc image list ubuntu:
lxc image info ubuntu:22.04
lxc image copy ubuntu:22.04 local: --alias my-ubuntu
lxc image delete my-ubuntu

lxc publish mycontainer --alias my-snapshot   # snapshot container as image
```

## Snapshots

```bash
lxc snapshot mycontainer snap0
lxc snapshot mycontainer snap1 --stateful      # with memory state (containers)
lxc info mycontainer                            # shows snapshot list
lxc restore mycontainer snap0
lxc delete mycontainer/snap0
```

## Storage pools

```bash
lxc storage list
lxc storage show default
lxc storage create fast btrfs source=/dev/sdb
lxc storage volume list default
lxc storage volume create default myvolume
lxc config device add mycontainer vol1 disk pool=default source=myvolume path=/data
```

## Networking

```bash
lxc network list
lxc network show lxdbr0
lxc network create mybr ipv4.address=10.10.10.1/24 ipv4.nat=true ipv6.address=none
lxc network attach mybr mycontainer eth1
lxc network detach mybr mycontainer eth1
```

## Clustering

```bash
lxd init --preseed < cluster-preseed.yaml
lxc cluster list
lxc cluster show node1
lxc cluster remove node3
lxc launch ubuntu:22.04 mycontainer --target node2   # pin to specific member
```

## Debugging

```bash
lxc monitor                          # live event stream
lxc info --show-log mycontainer      # console log
lxc console mycontainer              # attach to console
journalctl -u snap.lxd.daemon        # LXD daemon logs (snap install)
journalctl -u lxd                    # LXD daemon logs (deb install)
```

## Common patterns

- Use profiles to share config across groups of instances
- Set `boot.autostart=false` on instances used only for testing
- Prefer unprivileged containers; enable `security.nesting` for Juju/Docker inside LXD
- Use `lxc copy` to clone instances for fast environment duplication
- LXD uses `subuid`/`subgid` for UID mapping — check `/etc/subuid` if permission errors occur inside containers
