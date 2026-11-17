# Control who sees which repositories

**Goal:** private code stays visible only to people allowed to read it.

Harvest checks access at the source instead of keeping its own lists:

- When someone adds a repository, Harvest tries to read it anonymously. If that works, the repository is **public** and everyone in Harvest can use it.
- Otherwise it is **private**. The person who added it, using their own SSH key or platform token, gets access. Another user gets access by adding the same repository URL (in **Knowledge** or **Code**) with their own credentials: Harvest checks that they can read it, grants access, and reuses the existing index.
- Access is checked again every hour for grants older than 24 hours. Someone who loses access upstream loses it in Harvest.
- Administrators see every repository.

So to give someone access, give them access on GitHub, GitLab, Gitea, Launchpad or your git server; they add their credentials under **Settings → Git access** and add the repository URL. To remove access, remove it upstream: Harvest drops it at the next check.

A repository a user cannot see does not appear in Knowledge, chat answers, code search or Code mode for them.
