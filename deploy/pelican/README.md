# Running Paracord on Pelican

The Paracord egg builds straight from GitHub. Installing a server clones this
repo and compiles it on the node, so there's nothing to upload by hand. Data is
kept in a MySQL/MariaDB database from Pelican's **Databases** tab.

## 1. Import the egg

In the Pelican admin area: **Eggs → Import → URL**, and paste:

```
https://raw.githubusercontent.com/Just-Cole/paracord/main/deploy/pelican/egg-paracord.json
```

Already imported an older version? Import again from the same URL to update it.

## 2. Create the server

1. Create a server from the **Paracord** egg and give it **one port** (allocation).
   Paracord listens on that port automatically.
2. Give it at least **2 GB of memory** and **5 GB of disk**. The install step
   compiles Rust and keeps its build cache in `.build/` (about 1.5–2 GB) so later
   updates are much faster. Running Paracord itself needs very little.
3. Wait for the install to finish. The first one takes roughly 5–15 minutes
   depending on the CPU. You can watch it in the server console.

What the install does: see [install.sh](install.sh). It runs in the official
`rust:1-bookworm` image, downloads Trunk, builds the web client and a static
Linux binary, and installs `paracord` (plus a `VERSION` file) into the server.
Deleting `.build/` is safe; the next reinstall just takes longer.

## 3. Give it a database

1. In the server's **Databases** tab, create a database (any name, connections
   from `%`).
2. Open the server's **Startup** tab and copy the values in:

| Startup setting | Where to find it |
| --- | --- |
| Database host | the database's **Endpoint**, without the `:3306` |
| Database port | `3306` (the number after the colon in Endpoint) |
| Database name | the database name (`s<number>_…`) |
| Database username | the username (`u<number>_…`) |
| Database password | click to reveal it, and paste it exactly as shown |

3. Start the server. Paracord creates its tables on first start, and it's up
   when the console says `Paracord listening on`.

If the console keeps saying **"Can't reach the database at …"**, the host name
the panel shows (for example `pelican-database-1`) isn't reachable from inside
the server's container. Try the node's IP address or `172.18.0.1` as the host
instead.

Back up the database with Pelican's database backups (or `mysqldump`).
Reinstalling the server never touches it.

## 4. Point the domain at it

In the server's **Startup** tab:

- **Public site address**: your `https://` address, e.g. `https://paracord.crafternodes.space`
- **Secure cookies**: `1`

Then in Cloudflare either route a tunnel hostname to
`http://localhost:<allocated port>`, or use a proxied DNS record: an `A` record
to the VPS IP, an **Origin Rule** that rewrites the destination port to the
allocated port, and a **Configuration Rule** that sets SSL to **Flexible** for
that hostname. Make sure the VPS firewall allows the port.

## Updating

Push changes to GitHub, then press **Reinstall** on the server (Settings tab).
It rebuilds from the branch's latest commit and replaces only the binary.
Database changes apply automatically on the next start.

To build from a fork or another branch, change **Git repository** / **Git branch**
in the Startup tab before reinstalling.

## Voice (later)

Voice will run as a second Pelican server running LiveKit. It needs its own TCP
port plus a UDP port range allocated for media.
