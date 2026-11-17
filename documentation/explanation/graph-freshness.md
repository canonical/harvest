# Keeping the code graph fresh

The code graph describes a commit. A coding session changes files minute by minute, and the default branch moves as others merge. Harvest handles both without re-indexing constantly.

## The default branch

A background **keeper** checks each code project's default branch every `reindex_hours` (default 6). If the commit moved, it indexes the new commit as a new version. Summaries and embeddings are keyed by the symbol's content, so unchanged symbols reuse them: only changed code costs model calls.

## The session's own changes

Re-indexing on every edit would be slow and wasteful. Instead:

- **`search_workspace`** runs `git grep` in the container, so the agent always finds current text, including uncommitted edits;
- graph results pass through a **workspace overlay**: when a result points at a file that differs in the workspace, the result carries a note saying so, and the agent reads the file from the workspace instead of trusting the graph.

The graph stays the map of structure; the workspace is the truth for content. The agent uses the first to know where to look and the second to see what is there.
