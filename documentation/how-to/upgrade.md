# Upgrade Harvest

```bash
sudo snap refresh harvest
harvest status
```

The server applies database migrations on start. Check the schema version with `knowledge-server --config <file> schema-version` (snap: `sudo harvest.admin schema-version`).

In a cluster, refresh one node at a time and wait for `GET /health/ready` before the next.

When a release changes the code graph's parser, Harvest marks indexed versions as stale and re-indexes them on the next ingestion.
