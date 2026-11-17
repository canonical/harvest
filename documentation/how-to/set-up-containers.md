# Set up project containers

**Goal:** sessions start with the tools and dependencies the project needs.

Harvest builds one image per project: Ubuntu, the repository at `/workspace`, then a setup script run as root from the repository root (time limit 30 minutes). Sessions and previews start from that image.

The setup script comes from, in order:

1. `.harvest/setup.sh` in the repository, if present and not empty;
2. otherwise **Container setup** on the project page;
3. otherwise a script Harvest derives from the repository's files: Python packages, tools and dependency groups from `pyproject.toml` and requirements files, npm, pnpm or yarn dependencies, Rust and Go toolchains, `make`. A failure of this derived script does not fail the image.

```bash
apt-get update
apt-get install -y nodejs npm
npm ci
```

Saving a new script rebuilds the image. Warm containers are replaced as the new image is ready; running sessions keep their container.

Each new session also runs the derived preparation as a background job, so tools the image lacks are installed while the agent starts reading code. It shows in the session's **Jobs** tab.

When an install command succeeds in a session, Harvest remembers it. The project page lists these commands under **Container setup** with **Add them to the setup script**.

The agent can save a working script itself with its `save_setup_script` tool, for example after it had to install something: ask it to *"save what you installed as the setup script"*.

When setup fails, the project page shows **The last setup failed** with the end of its log. Fix the script and save it again.
