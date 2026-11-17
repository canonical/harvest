# How previews stay isolated

A preview runs code written minutes ago, possibly by an agent, and shows it inside Harvest. That code must not be able to act as the viewer in Harvest, and viewers must not disturb each other.

## One origin per preview

Each preview is served at its own host name, `p-<id>.<preview_domain>`. Browsers isolate origins: a page on the preview's origin cannot read Harvest's cookies, call Harvest's API as the viewer, or read another preview. Serving previews under a path of Harvest's own domain would give the previewed app the viewer's Harvest session.

Own origins also mean apps work unmodified: absolute paths, client-side routing, service workers and websockets behave as they do locally. That is why the runtime contract is just "listen on `$PORT`".

## Signing in to a preview

The preview origin cannot see Harvest's session. Harvest hands it a one-time ticket, valid one minute, which the preview exchanges for its own cookie. Inside Harvest's frame that cookie is third-party, so over HTTPS it is `SameSite=None; Secure; Partitioned`: allowed in the frame, but kept separate from the same site opened elsewhere. Over plain HTTP browsers refuse such cookies, so previews open in a new tab instead.

## Framing

Harvest rewrites the app's framing headers to `frame-ancestors <Harvest's origin>`. The preview can be framed by Harvest and by nothing else.

## A copy per viewer

The session owner sees the app running in their session. Everyone else gets a copy of the container, started from the session at that moment. A viewer who deletes all the cards, or crashes the app, does so in their own copy. Copies are capped per preview and removed when idle.
