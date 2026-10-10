#!/bin/bash
set -euo pipefail

LAB="${LAB_DIR:-$HOME/.cache/harvest-ha-lab}"
PG_A="${PG_A:-harvest-pg-a}"
PG_B="${PG_B:-harvest-pg-b}"
LB="${LB:-harvest-ha-lab}"
HOST_IP="${HOST_IP:-$(ip -4 addr show lxdbr0 | awk '/inet /{print $2}' | cut -d/ -f1)}"
DB_PASSWORD="${DB_PASSWORD:-harvest-ha-password}"

mkdir -p "$LAB/certs"
ip_of() { lxc list "$1" -c 4 --format csv | cut -d' ' -f1; }
A_IP=$(ip_of "$PG_A")
B_IP=$(ip_of "$PG_B")
LB_IP=$(ip_of "$LB")

if [ ! -f "$LAB/certs/ca.pem" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=harvest-ha-lab-ca" \
    -keyout "$LAB/certs/ca.key" -out "$LAB/certs/ca.pem" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -subj "/CN=harvest-postgres" \
    -keyout "$LAB/certs/server.key" -out "$LAB/certs/server.csr" 2>/dev/null
  printf "subjectAltName=IP:%s,IP:%s,DNS:localhost\n" "$A_IP" "$B_IP" > "$LAB/certs/san.ext"
  openssl x509 -req -in "$LAB/certs/server.csr" -CA "$LAB/certs/ca.pem" -CAkey "$LAB/certs/ca.key" \
    -CAcreateserial -days 30 -extfile "$LAB/certs/san.ext" -out "$LAB/certs/server.pem" 2>/dev/null
fi

install_postgres() {
  local node="$1"
  lxc exec "$node" -- cloud-init status --wait >/dev/null 2>&1 || true
  for attempt in 1 2 3 4 5; do
    if lxc exec "$node" -- bash -c "command -v pg_ctlcluster >/dev/null || (apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq postgresql-16 postgresql-16-pgvector >/dev/null)"; then
      break
    fi
    sleep 10
  done
  lxc file push "$LAB/certs/server.pem" "$node/etc/postgresql/16/main/server.pem"
  lxc file push "$LAB/certs/server.key" "$node/etc/postgresql/16/main/server.key"
  lxc exec "$node" -- bash -c "chown postgres:postgres /etc/postgresql/16/main/server.* && chmod 600 /etc/postgresql/16/main/server.key"
  lxc exec "$node" -- bash -c "cat > /etc/postgresql/16/main/conf.d/harvest-ha.conf <<CONF
listen_addresses = '*'
ssl = on
ssl_cert_file = '/etc/postgresql/16/main/server.pem'
ssl_key_file = '/etc/postgresql/16/main/server.key'
wal_level = replica
max_wal_senders = 10
hot_standby = on
max_connections = 200
CONF
grep -q 'hostssl all all 10.0.0.0/8' /etc/postgresql/16/main/pg_hba.conf || cat >> /etc/postgresql/16/main/pg_hba.conf <<HBA
hostssl all all 10.0.0.0/8 scram-sha-256
hostssl replication all 10.0.0.0/8 scram-sha-256
host all all 10.0.0.0/8 scram-sha-256
host replication all 10.0.0.0/8 scram-sha-256
HBA"
}

install_postgres "$PG_A"
lxc exec "$PG_A" -- systemctl restart postgresql@16-main
lxc exec "$PG_A" -- sudo -u postgres psql -qc "DO \$\$BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'harvest') THEN CREATE ROLE harvest LOGIN REPLICATION CREATEDB PASSWORD '$DB_PASSWORD'; END IF; END\$\$;"
lxc exec "$PG_A" -- sudo -u postgres psql -qtc "SELECT 1 FROM pg_database WHERE datname = 'harvest'" | grep -q 1 || \
  lxc exec "$PG_A" -- sudo -u postgres createdb -O harvest harvest
lxc exec "$PG_A" -- sudo -u postgres psql -q -d harvest -c "CREATE EXTENSION IF NOT EXISTS pg_trgm; CREATE EXTENSION IF NOT EXISTS vector;"

install_postgres "$PG_B"
if ! lxc exec "$PG_B" -- test -f /var/lib/postgresql/16/main/standby.signal; then
  lxc exec "$PG_B" -- systemctl stop postgresql@16-main
  lxc exec "$PG_B" -- bash -c "rm -rf /var/lib/postgresql/16/main && sudo -u postgres env PGPASSWORD='$DB_PASSWORD' pg_basebackup -h $A_IP -U harvest -D /var/lib/postgresql/16/main -R -X stream -c fast"
  lxc exec "$PG_B" -- systemctl start postgresql@16-main
fi

cat > "$LAB/env" <<ENV
HOST_IP=$HOST_IP
PG_A_IP=$A_IP
PG_B_IP=$B_IP
LB_IP=$LB_IP
DB_URL=postgres://harvest:$DB_PASSWORD@$A_IP:5432,$B_IP:5432/harvest?target_session_attrs=read-write&sslmode=require&connect_timeout=3
CA_FILE=$LAB/certs/ca.pem
ENV
echo "provisioned: primary $A_IP, replica $B_IP, load balancer $LB_IP, nodes on $HOST_IP"
