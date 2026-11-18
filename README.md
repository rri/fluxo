<p align="center">
  <a href="https://fluxo.dev"><img src="docs/readme-banner.svg" width="540" alt="fluxo: critical-path planning"></a>
</p>

<p align="center"><a href="https://fluxo.dev">fluxo.dev</a></p>

Critical-path planning: accelerate goals by focusing on the work that decides when they finish,
and on removing the risk and ambiguity along it.

- **Goals** break down into goals and tasks. Every goal feeds a bigger one, here or not.
- **Forecasts** come from a Monte Carlo simulation of estimates, clarity, coverage and risks:
  P50/P85 dates, each step's criticality, and the most likely critical path.
- **Focus** ranks what to do next by how much expected delay it removes.

Read more:

- How each page and state should behave, with the rule IDs the tests cite: [`docs/spec.md`](docs/spec.md)
  (and which tests cover each rule: [`docs/spec-coverage.md`](docs/spec-coverage.md))
- How forecasts and Focus gains are calculated: [`docs/forecasting.md`](docs/forecasting.md)
- Decisions made while building it: [`docs/implementation-plan.md`](docs/implementation-plan.md)
- What it defends against, and the risks it accepts: [`docs/threat-model.md`](docs/threat-model.md)

## Layout

| Path | What |
|---|---|
| [`crates/fluxo-sim`](crates/fluxo-sim) | The simulation engine (pure, no I/O) |
| [`crates/fluxo-core`](crates/fluxo-core) | Domain rules: access, usernames, working calendars |
| [`crates/fluxo-web`](crates/fluxo-web) | Web app: routes, pages (askama), sign-in, forecasts, Focus |
| [`crates/fluxo`](crates/fluxo) | The server binary |
| [`migrations/`](migrations) | SQLite schema |
| [`tests/browser/`](tests/browser) | Playwright tests, with and without JavaScript |
| [`scripts/`](scripts) | Refresh the sqlx query metadata; seed a demo goal |
| [`deploy/`](deploy) | Server files and how deploys work |

## Development

Requires Rust 1.88+. The browser tests also need Node.

```sh
cargo run -p fluxo            # http://127.0.0.1:8080, data in ./data
cargo test --workspace        # Rust tests
cargo clippy --workspace --all-targets
cargo fmt --check
```

On first start, fluxo logs a one-time link to create the Administrator account:

```text
Create the Administrator account: http://127.0.0.1:8080/setup?token=…
```

A new link is printed on every start until setup is done. Set `FLUXO_SETUP_TOKEN` to keep it the same.

- **Browser tests** run Chromium (with and without JavaScript) and Firefox against a built binary:

  ```sh
  cargo build -p fluxo
  cd tests/browser && npm ci && npx playwright install chromium firefox
  npm test                                    # against target/debug/fluxo
  FLUXO_BIN=../../target/release/fluxo npm test   # or another build
  ```

- **Changing SQL?** Run [`scripts/sqlx-prepare.sh`](scripts/sqlx-prepare.sh) to refresh the query
  metadata in `.sqlx/`, so builds work without a database.
- **Sample data:** [`scripts/seed-demo.sh`](scripts/seed-demo.sh) sets up a brand-new server and
  enters a demo goal through its own forms.
- **Testing from another machine over plain HTTP?** Set `FLUXO_SECURE_COOKIES=false`.

Test and dev builds optimise dependencies, because password hashing is about 25 times slower without
optimisation. Release builds use full link-time optimisation; see [`Cargo.toml`](Cargo.toml).

## Running in production

```sh
rustup target add x86_64-unknown-linux-musl   # once (CI also installs musl-tools)
cargo build --release -p fluxo --target x86_64-unknown-linux-musl
```

The result is one static binary that needs nothing else on the server.

fluxo speaks plain HTTP on localhost. Put a reverse proxy with HTTPS in front of it.
How https://fluxo.dev is set up (Caddy and systemd), and deployed by
[CI](.github/workflows/ci.yml) on every push to `main`, is in [`deploy/README.md`](deploy/README.md).

```text
fluxo [serve]        run the server (the default)
fluxo backup PATH    copy the database to PATH (a consistent snapshot; safe while running)
fluxo --version      print the version and the commit it was built from
```

### Configuration

Settings come from a TOML file: `$FLUXO_CONFIG` if set, else `/etc/fluxo/fluxo.toml` if it exists,
else the built-in defaults for local use. Environment variables override individual settings.
See [`deploy/fluxo.toml`](deploy/fluxo.toml) for the file fluxo.dev uses.

| Setting | Environment | Default |
|---|---|---|
| `listen` | `FLUXO_LISTEN` | `127.0.0.1:8080` |
| `public_url` | `FLUXO_PUBLIC_URL` | none (links use the Host) |
| `data_dir` | `FLUXO_DATA_DIR` | `./data` |
| `key_file` | `FLUXO_KEY_FILE` | `<data_dir>/secret.key` |
| `secure_cookies` | `FLUXO_SECURE_COOKIES` | `true` |
| `trusted_proxies` | `FLUXO_TRUSTED_PROXIES` | `["127.0.0.1", "::1"]` |
| `backup_keep` | `FLUXO_BACKUP_KEEP` | `7` nightly snapshots |
| `setup_token` | `FLUXO_SETUP_TOKEN` | random each start |

Logging is set with `RUST_LOG` (for example `RUST_LOG=debug`).

### Data and backups

`data_dir` holds:

- `fluxo.db`: the SQLite database
- `secret.key`: encrypts 2FA secrets. It is created on first start unless `key_file` names one.
- `backups/`: nightly snapshots, keeping the newest `backup_keep`

Back up the key separately from the database. Without it, 2FA secrets can't be read.

## License

AGPL-3.0-only. See [`LICENSE`](LICENSE).
