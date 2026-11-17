# Coding sessions

A coding session gives the agent a real development environment: a Linux container with the repository on its own branch, where it can install packages, run tests and start the app, without touching anyone else's work.

## Why containers

Code an agent writes must be run to be trusted. Running it on the Harvest server, or on a developer's laptop, would be unsafe and unrepeatable. An LXD container per session is isolated, disposable and a full Ubuntu system, so anything that works on a developer machine works there.

## Fast starts

Starting from scratch would mean cloning and installing every time. Instead, each project has an **image**: Ubuntu, the repository and the [setup script](../how-to/set-up-containers.md) already run. Harvest keeps a few **warm containers** from that image, so a new session usually starts in seconds by claiming one and checking out its branch. Images rebuild when the setup script changes; warm pools shrink when a project is unused.

## Credentials stay outside

The container never holds your git credentials or model keys. The agent's commands run as root inside it, so anything placed there could leak through the code it runs. Pushes and change requests happen on the Harvest server: it takes a git bundle of the session's commits out of the container and pushes it with your credentials.

## One session, many views

A session is a container and a branch. Chats, change views and previews are views on it, arranged as tabs and splits. Several chats share the same files, so you can keep separate conversations for separate concerns without separate copies of the code.

## Ending

Sessions end when their change request is merged, after a day idle, or by hand. Ending deletes the container; what was pushed stays.

## Work the agent does not have to remember

Some steps matter too much to depend on the agent choosing to take them, so Harvest takes them itself, as background jobs:

- **Preparing the environment.** Installing toolchains and dependencies is mechanical and slow. Harvest derives it from the repository's files and runs it when a session starts, so the agent's first minutes go to the code, not to `apt-get`.
- **Running the checks.** A change is only done when the repository's own checks pass. Harvest runs them after every turn that changed files and shows the result on the session, whether or not the agent ran them.
- **Reviewing the change.** An agent checking its own work tends to agree with itself. A reviewer with a fresh context and read-only tools reads each new commit against the request and reports only real problems.

The agent can also start long commands in the background and keep working. Finished jobs are reported in its next tool result or turn, so it never has to poll. All of these share one mechanism: a session job, run by a supervisor in the container and recorded in the database. New kinds of work plug into it rather than adding new machinery.
