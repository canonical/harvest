---
name: ceph
description: Operate Ceph distributed storage — OSDs, pools, CRUSH maps, RBD images, CephFS, RadosGW, health checks, and recovery
---

# Ceph

Ceph is a distributed storage system providing object (RADOS), block (RBD), and file (CephFS) storage. All data is stored in OSDs; Monitors maintain the cluster map.

## Core components

- **Monitor (MON)**: maintains cluster map and quorum (deploy odd number ≥ 3)
- **OSD**: stores data, handles replication and recovery (one per disk recommended)
- **Manager (MGR)**: exposes metrics and hosts dashboard/modules
- **MDS**: metadata server for CephFS (required only for CephFS)
- **RGW**: RADOS Gateway — S3/Swift-compatible object storage endpoint

## Cluster health

```bash
ceph status                       # full overview
ceph health detail                # verbose health warnings/errors
ceph df                           # used space per pool
ceph osd df                       # per-OSD usage
ceph osd stat
ceph mon stat
ceph mgr stat
```

## OSD operations

```bash
ceph osd tree                     # topology view
ceph osd ls
ceph osd out 3                    # mark OSD out (data migrates away)
ceph osd in  3                    # mark OSD in
ceph osd down 3
ceph osd purge 3 --yes-i-really-mean-it   # fully remove OSD

# Replacing a failed OSD (typical flow)
systemctl stop ceph-osd@3
ceph osd out 3
ceph osd purge 3 --yes-i-really-mean-it
# physically replace disk
ceph-volume lvm create --data /dev/sdX
```

## Pools

```bash
ceph osd pool ls detail
ceph osd pool create mypool 32       # 32 PGs (replicated, default)
ceph osd pool create ecpool 32 32 erasure   # erasure-coded
ceph osd pool set mypool size 3             # replication factor
ceph osd pool set mypool min_size 2
ceph osd pool get mypool all
ceph osd pool delete mypool mypool --yes-i-really-really-mean-it

ceph osd pool application enable mypool rbd    # mark pool for RBD use
```

## PG (Placement Group) tuning

```bash
ceph osd pool get mypool pg_num
ceph osd pool set mypool pg_num 64
ceph osd pool set mypool pgp_num 64
ceph pg stat
ceph pg dump | grep scrubbing
```

## RADOS Block Device (RBD)

```bash
rbd create mypool/myimage --size 10G
rbd ls mypool
rbd info mypool/myimage
rbd resize mypool/myimage --size 20G
rbd rm mypool/myimage

rbd map mypool/myimage            # map to /dev/rbdX on this host
rbd showmapped
rbd unmap /dev/rbd0

rbd snap create mypool/myimage@snap1
rbd snap ls mypool/myimage
rbd snap rollback mypool/myimage@snap1
rbd snap purge mypool/myimage
rbd snap rm mypool/myimage@snap1

rbd export mypool/myimage /tmp/backup.img
rbd import /tmp/backup.img mypool/restored
```

## CephFS

```bash
ceph fs ls
ceph fs status
ceph mds stat

# Create a filesystem
ceph osd pool create cephfs_data 32
ceph osd pool create cephfs_meta 32
ceph fs new myfs cephfs_meta cephfs_data

# Mount (kernel client)
mount -t ceph mon1:/ /mnt/cephfs -o name=admin,secretfile=/etc/ceph/admin.secret

# Mount (FUSE)
ceph-fuse /mnt/cephfs
```

## CRUSH map

```bash
ceph osd crush tree
ceph osd crush dump
ceph osd crush rule ls
ceph osd crush rule dump
getcrushmap -o crushmap.bin
crushtool -d crushmap.bin -o crushmap.txt   # decompile
# edit crushmap.txt
crushtool -c crushmap.txt -o crushmap-new.bin
setcrushmap -i crushmap-new.bin
```

## Authentication (CephX)

```bash
ceph auth ls
ceph auth get client.admin
ceph auth get-or-create client.myapp mon 'allow r' osd 'allow rw pool=mypool'
ceph auth caps client.myapp osd 'allow rw pool=mypool'
ceph auth del client.myapp
```

## Scrubbing and repair

```bash
ceph pg scrub <pg-id>
ceph pg deep-scrub <pg-id>
ceph osd scrub 3

# Repair inconsistent PGs
ceph health detail | grep inconsistent
ceph pg repair <pg-id>
```

## Recovery tuning

```bash
# Throttle recovery to reduce impact on client I/O
ceph osd set-backfillio-ratio 0.3
ceph tell osd.* injectargs '--osd-max-backfills 1'
ceph tell osd.* injectargs '--osd-recovery-max-active 1'

ceph osd set noout        # prevent OSDs from being marked out during maintenance
ceph osd unset noout
ceph osd set norebalance
ceph osd unset norebalance
```

## Common patterns

- Always `ceph osd set noout` before node maintenance
- Monitor `ceph -w` in a separate terminal during upgrades or recovery
- Prefer `ceph-volume lvm` for new OSD provisioning (replaces `ceph-disk`)
- PG count: aim for ~100 PGs per OSD; formula: `(OSDs × 100) / replicas`, round to power of 2
- Use `ceph osd df` to identify OSDs near capacity before they reach `nearfull` ratio
