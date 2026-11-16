CREATE SEQUENCE IF NOT EXISTS cluster_lease_tokens;

CREATE TABLE IF NOT EXISTS cluster_leases (
    key         text PRIMARY KEY,
    holder      text        NOT NULL,
    token       bigint      NOT NULL,
    acquired_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS fencing_tokens (
    key   text PRIMARY KEY,
    token bigint NOT NULL
);
