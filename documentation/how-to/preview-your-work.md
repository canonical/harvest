# Preview a web app, an API or a command-line tool

**Goal:** try the session's work while it is being built.

The easiest way: ask the agent, *"set up a web preview"*. It finds the start command and port, starts the app, checks it answers, and fixes what stops it.

To do it yourself, open a **Preview** tab and select **New preview**, then pick what people should try:

| Kind | Fields | Shows |
|---|---|---|
| **Web app** | port, start command | the site inside Harvest, with path, reload, width and **Open in new tab** |
| **HTTP API** | port, start command | a request form: method, path, headers, body, and the response |
| **Command-line tool** | none | a command box that runs in a copy of the workspace |

![New preview](../images/preview-new.png)

- Harvest sets `PORT` to the port and `HOST` to `0.0.0.0`. The app must listen on `0.0.0.0`.
- The start command should only start the app (`npm run dev -- --host 0.0.0.0 --port $PORT`); install and build in the setup script or a chat first. Leave it empty if the app already runs.
- Each preview of a session needs its own port.

From the preview's **⋯** menu: **Restart the app**, **View start log**, **Delete preview…**. **Keep this preview for another day** delays its expiry.

![Preview actions](../images/session-preview-actions.png)

Web previews need a [preview domain](serve-previews.md). Previews expire 24 hours after last use and end with their session.
