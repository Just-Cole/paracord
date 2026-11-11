# Running Paracord on Pelican

The Paracord egg builds straight from GitHub. Installing a server clones this
repo and compiles it on the node, so there's nothing to upload by hand.

## 1. Import the egg

In the Pelican admin area: **Eggs → Import → URL**, and paste:

```
https://raw.githubusercontent.com/Just-Cole/paracord/main/deploy/pelican/egg-paracord.json
```

## 2. Create the server

1. Create a server from the **Paracord** egg and give it **one port** (allocation).
   Paracord listens on that port automatically.
2. Give it at least **2 GB of memory** and **5 GB of disk**. The install step
   compiles Rust and keeps its build cache in `.build/` (about 1.5–2 GB) so later
   updates are much faster. Running Paracord itself needs very little.
3. Wait for the install to finish. The first one takes roughly 5–15 minutes
   depending on the CPU. You can watch it in the server console.
4. Start the server. It's up when the console says `Paracord listening on`.

The database is `data/paracord.db` in the server's files. Include it in your
backups.

What the install does: see [install.sh](install.sh). It runs in the official
`rust:1-bookworm` image, downloads Trunk, builds the web client and a static
Linux binary, and installs `paracord` (plus a `VERSION` file) into the server.
Deleting `.build/` is safe; the next reinstall just takes longer.

## 3. Point the domain at it

With cloudflared on the VPS, route `paracord.obsidianinteractive.site` to
`http://localhost:<allocated port>` (or the node's IP and port). Then, in the
server's **Startup** tab:

- **Public site address**: `https://paracord.obsidianinteractive.site`
- **Secure cookies**: `1`
- **Trusted proxy IPs**: the address cloudflared connects from. If it runs on the
  VPS host, that's normally the Docker gateway `172.18.0.1`. If sign-ins all seem
  to come from one IP in the logs, this is the setting to fix.

## Updating

Push changes to GitHub, then press **Reinstall** on the server (Settings tab).
It rebuilds from the branch's latest commit and replaces only the binary;
`data/` is kept, and database changes apply automatically on the next start.

To build from a fork or another branch, change **Git repository** / **Git branch**
in the Startup tab before reinstalling.

## Voice (later)

Voice will run as a second Pelican server running LiveKit. It needs its own TCP
port plus a UDP port range allocated for media.
