# Paracord

A community chat app: communities with text channels, direct messages, live
updates, typing indicators and presence. Everything is Rust:

| Part | Folder | What it is |
| --- | --- | --- |
| Server | `server/` | Axum. Serves the API (`/api/...`), the live WebSocket (`/ws`) and the web client, all on one port. SQLite database in `data/paracord.db`. |
| Web client | `web/` | Leptos, compiled to WebAssembly with Trunk. Built into `web/dist/` and baked into the server binary for release builds. |
| Shared | `shared/` | Types and input rules both sides use, so the browser and server can't disagree. |

The first account created becomes owner of the **Paracord** home community
(`#general`, `#introductions`). Everyone who signs up joins it. People can also
create their own communities or join others from **Find communities** (the ⌕
button), and DM anyone they share a community with.

## Tools (one-time)

- Rust (`rustup`), with the WebAssembly target: `rustup target add wasm32-unknown-unknown`
- Trunk: `cargo install trunk --locked`

## Run it on this PC

```bash
cd web && trunk build --release && cd ..
```

```bash
cargo run -p paracord-server
```

Open <http://localhost:3000>. Stop with Ctrl+C.

While working on the web client, keep `trunk watch` running in `web/` in a second
terminal; debug builds of the server read `web/dist` from disk, so a browser
reload picks up changes. Changes to `server/` or `shared/` need the server restarted.

## Settings

All optional, set as environment variables:

| Variable | Default | Meaning |
| --- | --- | --- |
| `PORT` | `SERVER_PORT`, else `3000` | Port to listen on. Pelican sets `SERVER_PORT` for you. |
| `BIND` | `0.0.0.0` | Address to listen on. |
| `DATA_DIR` | `data` | Folder for `paracord.db`. Back this folder up. |
| `TRUST_PROXY` | `127.0.0.1,::1` | Proxies whose forwarded-IP headers are believed (used for sign-in rate limits). |
| `PUBLIC_ORIGINS` | none | Your public site address, e.g. `https://paracord.example.com`. |
| `SECURE_COOKIES` | `0` | `1` to mark the sign-in cookie HTTPS-only. Use it behind HTTPS. |
| `RUST_LOG` | `paracord=info` | Log detail. |

## Cloudflare Tunnel

The site, API and WebSocket now share **one** address and port, so a single
route is enough: `paracord.obsidianinteractive.site → http://localhost:3000`.
The old `api.` route is no longer used. When running behind the tunnel, set
`SECURE_COOKIES=1` and `PUBLIC_ORIGINS=https://paracord.obsidianinteractive.site`.

## Hosting on Pelican

Import the egg by URL and Pelican builds Paracord from this repo on install:
`https://raw.githubusercontent.com/Just-Cole/paracord/main/deploy/pelican/egg-paracord.json`.
Details in [deploy/pelican/README.md](deploy/pelican/README.md).

## Security notes

- Passwords are hashed with Argon2id. Sessions are random 256-bit tokens kept in
  an HttpOnly, SameSite=Lax cookie; the database stores only their SHA-256 hash.
- Changes from other websites are refused (Origin check), sign-up and sign-in
  are rate limited per IP, and message sending per user.

## What's next

Voice is planned through a separate LiveKit server (Paracord will hand out
LiveKit access tokens). Roles, invites, pins and reactions from the old app
are not rebuilt yet. The previous Node version is backed up in
`Desktop/Paracord-backup-2026-11-11`.
