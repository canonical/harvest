# Fix a preview that does not start

**Goal:** get a failing preview running.

When an app does not answer, the preview shows **The app could not start**, Harvest's diagnosis and the start log. Select **Ask the agent to fix this** to hand it over: the agent receives the same diagnosis.

| Diagnosis | Cause | Fix |
|---|---|---|
| `<command>` is not installed | the start command uses a program missing from the container (`npm: command not found`) | install it, add it to the [setup script](set-up-containers.md), start again |
| listens on 127.0.0.1 only | the app is bound to localhost | listen on `0.0.0.0` (use `$HOST`) |
| stopped before listening | the start command exited with an error | fix the error shown in the log |
| nothing listened in time | still installing or building, or on another port | install and build first; use `$PORT` |

Other symptoms:

- **Preview tab blank, no error:** the browser rejects the preview certificate. Open the preview address in a new tab to see the error; see [Use previews without a domain](previews-without-a-domain.md).
- **"Open the app" instead of a frame:** `preview_scheme` is `http`, so browsers refuse the sign-in cookie inside a frame. Use HTTPS.
- **Hot reload does not work:** the proxy does not forward websockets; see [Serve web previews](serve-previews.md).
