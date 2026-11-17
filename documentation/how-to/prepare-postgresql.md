# Prepare PostgreSQL

**Goal:** a database Harvest can use.

Harvest needs PostgreSQL 14 or later with two extensions: `vector` (pgvector, for semantic search) and `pg_trgm` (for name search). Without `vector`, Harvest runs with semantic search turned off.

## On the same host

```bash
harvest prepare-db-script | sudo bash -x
```

The script installs the `postgresql` snap if needed, creates the role and database, and prints the `database.url`. Options: `--db-name`, `--db-user`, `--db-host`, `--db-port`, `--db-password`, `--postgresql-snap`.

## On an existing server

As a superuser:

```sql
CREATE ROLE harvest LOGIN PASSWORD '<password>';
CREATE DATABASE harvest OWNER harvest;
\c harvest
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
```

Then:

```bash
sudo harvest database set-url 'postgresql://harvest:<password>@db.example.com:5432/harvest'
sudo harvest config apply
```

Create the extensions as a superuser first: the `harvest` role usually cannot.

For TLS to the database, set `database.ca_file` to the CA certificate path.

Harvest applies schema migrations on start (`database.migrate_on_start`, default `true`). To run them separately: `knowledge-server --config server.toml migrate`; to see the version: `knowledge-server --config server.toml schema-version`.
