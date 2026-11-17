# Index private repositories

**Goal:** let Harvest read repositories that need credentials.

Open **Settings → Git access**.

- **SSH key**: select **Generate a key** and add the public key to your account on the git host, or select **Use my own key** and paste a private key. Used for `git@…` and `ssh://` URLs.
- **Platform accounts**: select **Add account**, choose the platform, and enter the **Host**, your **Username** and an access token. Used for `https://` URLs and for opening change requests.

For Gerrit, the secret is your **HTTP password** from Gerrit's settings; for the other platforms it is an **access token** that can read the repository and, to open change requests, push branches and create them. A host that is not github.com, gitlab.com, launchpad.net or opendev.org needs a platform account so Harvest knows which platform it is.

Credentials are encrypted in the database and used only from the server: they never enter coding containers. Then add the repository as usual: [Index a repository](index-a-repository.md).
