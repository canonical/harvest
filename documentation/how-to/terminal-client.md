# Use Harvest from a terminal

**Goal:** chat with Harvest without a browser.

`harvest-tui` is a terminal chat client, built from the `tui` crate (`cargo build --release -p harvest-tui`).

```bash
harvest-tui --url https://harvest.example.com
```

Without credentials it prints a sign-in link: open it in a browser where you are signed in to Harvest and approve; the terminal continues by itself. Alternatives:

| Option | Environment variable |
|---|---|
| `--url` | `HARVEST_URL` (default `http://localhost:8080`) |
| `--email` and `--password` | `HARVEST_EMAIL`, `HARVEST_PASSWORD` (local accounts) |
| `--token` | `HARVEST_TOKEN` (an existing session token) |

Answers render as Markdown; diagrams are drawn as text.
