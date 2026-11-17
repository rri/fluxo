# Deploying fluxo.dev

Every push to `main` that passes CI is built as a static x86-64 binary (musl)
and installed on the server. These files are what's on the server; the
workflow is `.github/workflows/ci.yml`.

| File | On the server |
|---|---|
| `fluxo-deploy` | `/usr/local/sbin/fluxo-deploy` (root, 755) |
| `sudoers` | `/etc/sudoers.d/fluxo-deploy` (root, 440) |
| `authorized_keys` | one line appended to `~deploy/.ssh/authorized_keys` |
| `fluxo.service` | `/etc/systemd/system/fluxo.service` |
| `fluxo.toml` | `/etc/fluxo/fluxo.toml` |
| `Caddyfile` | added to `/etc/caddy/Caddyfile` |

fluxo listens on `127.0.0.1:8371`. Its data lives in
`/mnt/data/fluxo`: `fluxo.db`, `secret.key` (encrypts 2FA secrets; created on
first start) and `backups/`.

## How a deploy works

1. CI runs formatting, Clippy, every Rust test, the browser tests (Chromium
   with and without JavaScript, and Firefox), `cargo audit`, and the musl build.
2. The deploy job checks that its commit is still the newest on `main`, so a
   slow older run never replaces a newer deploy.
3. It joins the tailnet as a short-lived `tag:ci` node (SSH to the server and
   nothing else) and sends the binary over SSH as the
   `deploy` user.
4. fluxo's key can run only `fluxo-deploy`. It checks the checksum, runs the
   binary's `--version` as `fluxo`, stops fluxo, copies the database to
   `backups/pre-deploy-*.db` (the newest 5 are kept), installs the binary (the
   old one stays as `/usr/local/bin/fluxo.prev`) and starts it.
   If `/api/health` doesn't answer within 30 seconds, it puts the old binary
   and the database copy back and starts that. The script prints only its own
   steps, because they appear in the public Actions log.

## GitHub settings (repository rri/fluxo)

Create a `production` environment (deployments from `main` only) holding:

- `TS_OAUTH_CLIENT_ID`, `TS_OAUTH_SECRET`: a Tailscale OAuth client with the
  Auth Keys write scope and the tag `tag:ci`.
- `DEPLOY_SSH_KEY`: the private half of fluxo's deploy key (below).
- `DEPLOY_KNOWN_HOSTS`: the server's SSH host key, as `100.82.93.4 ssh-ed25519 …`.
  Take it from the server itself, not from `ssh-keyscan`:
  `ssh root@SERVER 'cat /etc/ssh/ssh_host_ed25519_key.pub' | awk '{print "100.82.93.4", $1, $2}'`.

Make fluxo's own deploy key, on your machine, not the server:

```sh
ssh-keygen -t ed25519 -f fluxo-deploy -C fluxo-deploy -N ''
# fluxo-deploy      → the DEPLOY_SSH_KEY secret
# fluxo-deploy.pub  → goes in authorized_keys on the server (below)
```

## Setting up the server (once)

As root, from a checkout of this directory:

```sh
# The service user and its data
useradd --system --home-dir /mnt/data/fluxo --no-create-home --shell /usr/sbin/nologin fluxo
install -d -m 700 -o fluxo -g fluxo /mnt/data/fluxo
install -D -m 644 fluxo.toml /etc/fluxo/fluxo.toml
install -m 644 fluxo.service /etc/systemd/system/fluxo.service
systemctl daemon-reload && systemctl enable fluxo

# Deploys: a second key for the existing deploy user, allowed to run only fluxo-deploy
sed "s|ssh-ed25519 REPLACE_WITH_PUBLIC_KEY fluxo-deploy|$(cat fluxo-deploy.pub)|" authorized_keys \
  | grep -v '^#' >> /var/lib/deploy/.ssh/authorized_keys
install -m 755 fluxo-deploy /usr/local/sbin/fluxo-deploy
install -m 440 sudoers /etc/sudoers.d/fluxo-deploy && visudo -c

# The web server
cat Caddyfile >> /etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile && systemctl reload caddy
```

### After the first deploy

The first deploy starts fluxo. Its log has the one-time link for creating the
Administrator account:

```sh
journalctl -u fluxo | grep 'Create the Administrator account'
```

Then copy `/mnt/data/fluxo/secret.key` somewhere safe off the server, once.
The backups hold the database but not the key, and without the key 2FA
secrets can't be read. People could still sign in with an Administrator's
recovery code and set up 2FA again.

## By hand

- Roll back to the previous binary:
  `cp -p /usr/local/bin/fluxo.prev /usr/local/bin/fluxo && systemctl restart fluxo`.
  If the newer version changed the database, stop fluxo first and copy the matching `pre-deploy-*.db` over `/mnt/data/fluxo/fluxo.db`
  (as `fluxo`, after deleting `fluxo.db-wal` and `fluxo.db-shm`).
- Deploy logs: `journalctl -t fluxo-deploy`.
- A snapshot any time: `runuser -u fluxo -- env FLUXO_CONFIG=/etc/fluxo/fluxo.toml fluxo backup /mnt/data/fluxo/backups/manual.db`.
- Backups: nightly snapshots (7 kept) and pre-deploy copies (5 kept) are in
  `/mnt/data/fluxo/backups`, on the same volume as the database. There is no
  off-site copy, so copy a snapshot off the server by hand when it matters:
  `scp 'root@SERVER:/mnt/data/fluxo/backups/nightly-*.db' .` (all 7).
  To restore one: stop fluxo, delete `fluxo.db-wal` and `fluxo.db-shm`, put the
  snapshot in place with `install -m 600 -o fluxo -g fluxo SNAPSHOT /mnt/data/fluxo/fluxo.db`
  and start fluxo.
