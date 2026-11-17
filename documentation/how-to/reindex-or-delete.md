# Re-index or delete a version

In **Knowledge**, select the repository and pick the version.

| Action | Button | Who |
|---|---|---|
| Re-read the version from git | ⟳ *Resynchronize this version* | anyone who can see the repository |
| Add versions | **+** *Ingest new versions* | anyone who can see the repository |
| Delete one version | **Delete version** | administrators |
| Delete the repository and every version | **Delete repository** | administrators |

A branch's version in a Code project is refreshed automatically: every `reindex_hours` (default 6), Harvest checks the branch and re-indexes it when its commit moved. Summaries and embeddings of unchanged code are kept.
