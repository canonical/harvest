# Change request platforms

| Platform | Name in Harvest | Detected for hosts | Push | Opens | Secret |
|---|---|---|---|---|---|
| GitHub | Pull request | `github.com` | branch over HTTPS | pull request via REST API | access token |
| GitLab | Merge request | `gitlab.com` | branch over HTTPS | merge request via REST API | access token |
| Gitea | Pull request | — | branch over HTTPS | pull request via REST API | access token |
| Gerrit | Gerrit change | `opendev.org`, `review.opendev.org` | `refs/for/<base>%topic=<branch>` with Change-Id trailers | the change created by the push | HTTP password |
| Launchpad | Merge proposal | `launchpad.net` | branch over SSH to your personal copy (`~<user>/…`); needs your SSH key | merge proposal via API | OAuth token |

Any other host needs a platform account under **Settings → Git access** so Harvest knows its platform.

Rules:

- Pushes run on the Harvest server, from a bundle of the session's commits; credentials never enter the container.
- Uncommitted changes block a proposal (*commit or discard the uncommitted changes first*).
- Harvest polls the change request's state; **merged** ends the session.
