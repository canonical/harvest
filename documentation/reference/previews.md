# Previews

## Kinds

| Kind | Port | Viewer gets |
|---|---|---|
| `web` | required | the app over HTTP and websockets, at its own origin |
| `api` | required | a request form; responses up to 256 KiB, 120 s timeout |
| `cli` | none | commands run in a copy of the workspace, 120 s timeout each |

## Runtime contract

Harvest starts the app in the container with:

```bash
HOST=0.0.0.0 PORT=<port> bash -lc '<start command>'   # from /workspace, in its own process group
```

| Rule | Detail |
|---|---|
| Listen address | `0.0.0.0:$PORT`. An app on `127.0.0.1` only is reported as *local only* |
| Start time | something must accept connections on the port within 90 s |
| Start command | starts the app only; install and build before. Empty: the app is assumed to be running already |
| Log | `/tmp/harvest-preview-<id>.log`, last 4,000 characters shown |
| Stop | `SIGTERM` to the process group, `SIGKILL` after 5 s |
| Ports | unique per session |

## Addresses

| Viewer | URL |
|---|---|
| Owner, in Harvest | `<preview_scheme>://p-<id>.<preview_domain>/` inside the Preview tab |
| Shared link | `https://harvest.example.com/#/preview/<id>`: Harvest's page for the preview, which frames the viewer's copy |

`<id>` is 32 hexadecimal characters.

## Sign-in to a preview origin

1. Harvest issues a one-time ticket, valid 60 seconds.
2. The preview origin exchanges it for a `harvest_preview` cookie, valid 24 hours: `HttpOnly; SameSite=None; Secure; Partitioned` over HTTPS, `SameSite=Lax` over HTTP.
3. Requests without a valid cookie are sent back to Harvest to sign in.

The app's `X-Frame-Options` and `frame-ancestors` are replaced with `frame-ancestors <Harvest's origin>`.

## Viewer copies

Viewers other than the owner each get a copy of the session container. At most `max_preview_copies` (default 5) per preview; the least recently used goes first. A copy idle for 2 hours is removed. **Fresh copy of the app** replaces the viewer's copy.

## Expiry

A preview is deleted `preview_ttl_hours` (default 24) after it was last opened. **Keep this preview for another day** resets the timer. Previews end with their session.
