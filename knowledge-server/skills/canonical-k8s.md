---
name: canonical-k8s
description: Operate Canonical Kubernetes (k8s snap) — bootstrap, node management, add-ons, networking, storage, upgrades, and troubleshooting
---

# Canonical Kubernetes

Canonical Kubernetes is the `k8s` snap — a fully-conformant, opinionated Kubernetes distribution. It uses a built-in datastore (dqlite) and ships networking, DNS, load balancer, and storage add-ons.

## Bootstrap a cluster

```bash
sudo snap install k8s --classic --channel 1.31/stable
sudo k8s bootstrap                    # single-node or first control plane
sudo k8s status --wait-ready          # wait until cluster is ready
```

### Bootstrap configuration (optional)

```bash
# Write config before bootstrap
cat > bootstrap-config.yaml <<EOF
cluster-config:
  network:
    enabled: true
  dns:
    enabled: true
  load-balancer:
    enabled: true
    l2-mode: true
    cidrs:
      - 10.0.0.200/29
  ingress:
    enabled: false
  local-storage:
    enabled: true
    local-path: /var/snap/k8s/common/rawfile-storage
EOF
sudo k8s bootstrap --file bootstrap-config.yaml
```

## kubectl access

```bash
sudo k8s kubectl get nodes
sudo k8s kubectl get pods -A
sudo k8s kubectl cluster-info

# Export kubeconfig for use with standard kubectl
mkdir -p ~/.kube
sudo k8s config > ~/.kube/config
kubectl get nodes
```

## Cluster membership

```bash
# On the control-plane node — generate a join token
sudo k8s get-join-token worker01
sudo k8s get-join-token worker01 --worker   # worker-only token

# On the new node
sudo snap install k8s --classic
sudo k8s join-cluster <token>

# List and remove nodes
sudo k8s kubectl get nodes
sudo k8s remove-node worker01
```

## Add-on management

```bash
sudo k8s enable dns
sudo k8s enable network
sudo k8s enable load-balancer
sudo k8s enable local-storage
sudo k8s enable ingress
sudo k8s enable metrics-server
sudo k8s enable gateway

sudo k8s disable ingress
sudo k8s status          # shows which add-ons are enabled/disabled
```

## Networking (Cilium under the hood)

```bash
# Load balancer — configure CIDRs
sudo k8s set load-balancer.cidrs="192.168.1.200/29"
sudo k8s set load-balancer.l2-mode=true

# DNS upstream
sudo k8s set dns.upstream-nameservers="8.8.8.8,8.8.4.4"

# Inspect network state
sudo k8s kubectl -n kube-system get pods -l app.kubernetes.io/name=cilium
```

## Storage (rawfile CSI / external)

```bash
# Local path provisioner (default add-on)
sudo k8s set local-storage.local-path=/data/k8s-storage
sudo k8s set local-storage.reclaim-policy=Delete

# Check storage classes
sudo k8s kubectl get storageclass
sudo k8s kubectl get pv,pvc -A
```

## Configuration management

```bash
sudo k8s get                         # show all current config
sudo k8s get cluster-config          # cluster-level settings
sudo k8s set <key>=<value>
sudo k8s set dns.enabled=true
sudo k8s set network.enabled=true
```

## Certificates and PKI

```bash
sudo k8s certs                        # show certificate expiry
sudo k8s refresh-certs                # renew expiring certs (non-disruptive)
```

## Upgrades

```bash
# Check available channels
snap info k8s | grep -A 20 channels

# Upgrade — one minor version at a time, control-plane first
sudo snap refresh k8s --channel 1.32/stable

# Always verify after upgrade
sudo k8s status --wait-ready
sudo k8s kubectl get nodes
```

## Troubleshooting

```bash
sudo k8s status
sudo k8s status --wait-ready --timeout 120

# Inspect services
sudo systemctl status snap.k8s.k8sd
sudo journalctl -u snap.k8s.k8sd -f
sudo journalctl -u snap.k8s.kubelet -f
sudo journalctl -u snap.k8s.containerd -f

# Run kubectl diagnostics
sudo k8s kubectl describe node <name>
sudo k8s kubectl get events -A --sort-by='.lastTimestamp'
sudo k8s kubectl top nodes
sudo k8s kubectl top pods -A

# Containerd / crictl
sudo k8s kubectl exec -it <pod> -- /bin/sh
sudo crictl ps                        # list containers at runtime level
sudo crictl logs <container-id>

# Kubeconfig and API access
sudo k8s config                       # raw kubeconfig output
```

## High availability (multi-control-plane)

```bash
# Three control-plane nodes = HA dqlite cluster
# Join second and third control-plane using a standard join token (no --worker flag)
sudo k8s get-join-token cp02
# On cp02:
sudo k8s join-cluster <token>

sudo k8s kubectl get nodes  # all three should show control-plane role
```

## Common patterns

- Run `sudo k8s status --wait-ready` after any cluster change before proceeding
- Drain a node before maintenance: `kubectl drain <node> --ignore-daemonsets --delete-emptydir-data`
- Use `kubectl get events -A --sort-by=.lastTimestamp` as a first diagnostic step
- Check add-on pod logs (`-n kube-system`) when an add-on is enabled but not working
- For persistent workloads use a StorageClass; `local-storage` add-on is not replicated
