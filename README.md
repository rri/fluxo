# fluxo

Critical-path planning: accelerate goals by focusing on the work that decides when they finish,
and on removing the risk and ambiguity along it. Hosted at https://fluxo.dev.

- **Goals** break down into sub-goals and tasks. Goals stay steady; the path beneath them changes.
- **Forecasts** come from a Monte Carlo simulation of estimates, clarity, coverage and risks:
  P50/P85 dates, each step's criticality, and the most likely critical path.
- **Focus** ranks what to do next by how much expected delay it removes.

## Layout

| Path | What |
|---|---|
| `crates/fluxo-sim` | The simulation engine (pure, no I/O) |
| `crates/fluxo-core` | Domain rules: access, usernames, working calendars |
| `crates/fluxo-web` | Web app: routes, pages (askama), sign-in, forecasts, Focus |
| `crates/fluxo` | The server binary (`fluxo serve`, `fluxo backup`, `fluxo --version`) |
| `migrations/` | SQLite schema |
| `tests/browser/` | Playwright tests, with and without JavaScript |
| `scripts/` | Refresh the sqlx query metadata; seed a demo goal |
| `deploy/` | Server files and how deploys work (see `deploy/README.md`) |
| `docs/implementation-plan.md` | Decisions and plan |
| `docs/spec.md` | Intended behaviour of every page and state, with rule IDs the tests cite |
| `docs/threat-model.md` | Threats, how each is handled, and accepted risks |

## Running locally

```sh
cargo run -p fluxo            # http://127.0.0.1:8080; the setup link is printed to the log
cargo test --workspace        # Rust tests
cargo build -p fluxo && (cd tests/browser && npm ci && npx playwright install chromium firefox && npm test)
```

Changing SQL? Run `scripts/sqlx-prepare.sh` to refresh the query metadata in `.sqlx/`.
For local testing over plain HTTP from another machine, set `FLUXO_SECURE_COOKIES=false`.

## License

AGPL-3.0-only. See `LICENSE`.
