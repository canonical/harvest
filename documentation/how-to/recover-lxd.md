# Recover LXD after it loses its configuration

**Symptom:** after a reboot, Code sessions fail with `LXD request failed (403 Forbidden)`, and `lxc list`, `lxc storage list` and `lxc config trust list` are empty, although `/var/snap/lxd/common/lxd/storage-pools/default/containers` still holds containers.

LXD's internal database came back nearly empty. Re-create its configuration and re-import the containers:

```bash
# 1. the bridge (use the previous subnet)
lxc network create lxdbr0 ipv4.address=10.55.167.1/24 ipv4.nat=true ipv6.address=none

# 2. the storage pool, in recovery mode, then import what is on disk
lxc storage create default dir source=/var/snap/lxd/common/lxd/storage-pools/default source.recover=true
sudo lxd recover          # answer yes to scan, yes to recover

# 3. the default profile
lxc profile device add default root disk path=/ pool=default
lxc profile device add default eth0 nic network=lxdbr0 name=eth0

# 4. start the containers
for c in $(lxc list -c n -f csv); do lxc start "$c"; done
```

Then let Harvest in again, either with a new trust token in `[lxd] trust_token` (Harvest joins on restart), or by trusting its existing certificate (stored in the `lxd_identity` table) with `lxc config trust add`.

Project images are not recovered. Delete Harvest's image records so the keeper rebuilds them:

```sql
DELETE FROM code_images;
```

To start from scratch instead, delete the containers and images, re-run `lxd init`, give Harvest a new trust token, and clear `code_images`, `code_containers` and the open sessions.
