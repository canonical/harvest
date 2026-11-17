# Connect GitHub, GitLab, Gitea, Gerrit or Launchpad

**Goal:** Harvest can read your private repositories, push your branches and open change requests as you.

Open **Settings → Git access → Platform accounts** and select **Add account**.

| Field | Value |
|---|---|
| Platform | GitHub, GitLab, Gitea, Gerrit or Launchpad |
| Host | e.g. `github.com`, `gitlab.example.com`, `review.opendev.org` |
| Username | your user on that host |
| Access token / HTTP password | a token that can read and push repositories and open change requests; for Gerrit, the HTTP password from its settings |

One account per host. Edit (✎) to replace the token; delete to remove it.

For `git@…` and `ssh://` URLs, also set an **SSH key** in the same section (generate one and add the public key to the platform, or paste your own).

Tokens and keys are encrypted with the server's key and used only by the server; they never enter coding containers. Hosts other than github.com, gitlab.com, launchpad.net and opendev.org need an account here so Harvest knows their platform.
