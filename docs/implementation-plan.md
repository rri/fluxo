# fluxo — implementation plan

Status: **approved** · decisions D1–D24 settled 2026-11-16 · Phase 0 done (engine: 500 nodes × 10k runs in 502 ms on 1 thread, 45 ms on 16) · Phase 1 next

This plan turns the round 2 mocks (`mocks/`) into a working application at https://fluxo.dev.
Every decision is recorded with its number (D1–D24) in sections 3 and 8.

---

## 1. Settled decisions (from the mock rounds)

**Product model**
- Goals break down into sub-goals and tasks. Tasks are always leaves. A child's output feeds its parent goal.
- A task has a known *what* and *why*. A goal has a known *why*, but its *what* is still open.
- A goal or task can feed several goals, and each one has a single **home parent**.
- Order between siblings comes from explicit dependencies. Siblings with no dependency between them run in parallel.
- The critical path is calculated, weighted by risk, and found by **Monte Carlo simulation**. It produces a criticality % for each node, and P50/P85 finish dates for each goal.
- How ambiguity is captured:
  - Each goal has a **coverage** level (low / partial / full). Anything short of full becomes an "unknown work" step in the forecast.
  - Each task has a **clarity** level (low / medium / high), which sets how wide its duration estimate is.
- Risks are their own objects, with a title, a likelihood % and an impact range in days. A risk is retired by setting its likelihood to 0%. Every likelihood change is kept as history.
- Estimates are in working days, in ½-day steps.

**People and access**
- Accounts use a username and password, with optional TOTP 2FA and personal recovery codes.
- The **creator owns** a goal or task. Ownership can be transferred, and the recipient must accept.
- People are invited to a goal or task as a **Contributor** (can edit) or **Watcher** (read-only). They must accept the invite. Roles are inherited by everything beneath, unless a more specific invite overrides them. Only the owner can delete, transfer or invite.
- **Administrators:**
  - The first user, created at instance setup, is an Administrator.
  - There can be several Administrators, and they can promote others. The last Administrator can't step down or delete their account.
  - An Administrator can issue a single-use recovery code that expires after 24 hours. Signing in with it forces a new password and turns 2FA off until the user sets it up again.
- When someone deletes their account, their goals and tasks are never deleted. Each item gets a **suggested** new owner, which the person leaving can change. Suggestions come from:
  1. the nearest owner up the item's home-parent chain;
  2. otherwise, the item's longest-standing Contributor;
  3. otherwise, the Administrators.

**Experience**
- **Focus** is the home page and the only view built around immediate work. It holds the ranked next actions (scored by "expected delay removed"), invitations and transfers waiting on the user, and Updates (notifications).
- Everywhere else, **goals are front and center**. A goal's page sets the goal apart at the top, and the graph puts the goal on top with work rising toward it.
- The top bar has Focus, Goals, the theme toggle and the account menu (Settings, Sign out). Administrator tools are a section of Settings, shown only to Administrators. Every page is a single column.
- Visual design:
  - Light and dark themes follow the OS, with a manual toggle.
  - The accent is indigo `#5B5BF7`.
  - Fonts come from the system font stack.
  - The logo is "converging rays", and the tab icon is all indigo.
- Pages are plain HTML5/CSS3 that work without JavaScript; vanilla JS enhances them.
- **Stack (your proposal):** vanilla HTML/CSS/JS frontend, Rust and SQLite backend.

---

## 2. Architecture

```
browser ──HTTPS──► reverse proxy (TLS) ──► fluxo (single Rust binary) ──► SQLite (WAL)
                                             ├─ web: routes, HTML templates, forms, JSON for enhancements
                                             ├─ core: domain rules, access control, handover rules
                                             ├─ sim: Monte Carlo engine (pure, no I/O)
                                             └─ jobs: simulation queue, expiry sweeps
```

- **Server-rendered HTML first.** Every page comes from the server as complete HTML, and every action is
  a normal `<form>` POST followed by a redirect. This keeps the mocks' promise that pages work without
  JavaScript. JavaScript then enhances pages: graph layout, live sliders, inline saves through small JSON
  endpoints, and filter tabs. **(D1)**
- **One binary, one database file.** Static assets (CSS, JS, logo) are embedded into the binary at build
  time. The only thing deployed is the binary, plus the database file it creates.
- **Crates in one Cargo workspace**: `fluxo-core` (domain + access rules), `fluxo-sim` (engine, pure and
  benchmarkable), `fluxo-web` (HTTP, templates, auth), `fluxo` (the binary: config, migrations, startup).
  **(D2)**
- **Simulation runs off the request path.** A change to a goal, task, dependency or risk marks the
  affected top-level goals as stale. A background worker re-simulates them, combining changes that
  arrive close together, and stores the results. Pages read the latest stored forecast and show when it
  was computed ("updated 4 min ago").

---

## 3. Backend technology choices (decided 2026-11-16)

All fifteen were decided as recommended; the alternatives are kept for the record.

| # | Decision | Decided | Not chosen |
|---|---|---|---|
| D1 | Rendering model | Server-rendered HTML + progressive enhancement | Static pages + JSON API (works without JS only with extra effort) |
| D2 | Code layout | Cargo workspace with 4 crates (above) | Single crate with modules |
| D3 | Web framework | **axum** (tokio, tower middleware) | actix-web; rocket |
| D4 | HTML templates | **askama** (templates compiled and type-checked) | maud (HTML written in Rust macros); minijinja (templates loaded at runtime) |
| D5 | SQLite access | **sqlx** with SQLite, queries checked at compile time, built-in migrations | rusqlite + refinery (synchronous, simpler) |
| D6 | Password hashing | **argon2id** (`argon2` crate), parameters set for roughly 50 ms per hash | scrypt |
| D7 | TOTP | **totp-rs** (RFC 6238, 30 s step, ±1 step allowed) | write it ourselves (small, but security-sensitive) |
| D8 | Sessions | A server-side session table. The cookie holds a random 256-bit ID (HttpOnly, Secure, SameSite=Lax). Sessions expire after 30 days idle. | Signed cookie sessions (can't be revoked per session, which the Settings "Sessions" list needs) |
| D9 | Form protection (CSRF) | SameSite=Lax cookies plus a per-session token in every form, checked by middleware | SameSite only |
| D10 | Secrets at rest | TOTP secrets encrypted with XChaCha20-Poly1305, using a key from an environment variable or file. Recovery codes stored only as hashes. | Store TOTP secrets in plain text, relying on disk encryption |
| D11 | Simulation parallelism | **rayon** over iterations, with a fixed seed per run so results can be reproduced | single thread at first |
| D12 | Hosting | One Linux VM, the binary under systemd, **Caddy** in front for automatic TLS | Fly.io or a similar container platform; nginx + certbot |
| D13 | Backups | Nightly `VACUUM INTO` snapshots plus a copy before every deploy, on the server's data volume | Continuous off-site replication to object storage (dropped 2026-11-16) |
| D14 | Testing | Unit tests in core/sim, property tests (proptest) for the graph and handover rules, HTTP integration tests against a temporary database, and Playwright for no-JS and JS smoke tests | Rust tests only |
| D15 | Minimum supported browsers | The last 2 versions of Firefox, Chrome and Safari. The mocks already use `color-mix()` and `:has`-era CSS. | Wider support (would need fallbacks) |

---

## 4. Data model (SQLite, proposed)

```
instance        (id=1, setup_completed_at, site_name)
users           (id, username UNIQUE, display_name, password_hash, totp_secret_enc NULL,
                 totp_enabled, is_administrator, created_at, deleted_at NULL)
user_calendar   (user_id, workdays_mask, time_zone, days_off JSON)
sessions        (id_hash PK, user_id, created_at, last_seen_at, user_agent, ip_hint)
recovery_codes  (id, user_id, code_hash, used_at NULL)                  -- personal codes
admin_codes     (id, user_id, issued_by, code_hash, expires_at, used_at, revoked_at)

nodes           (id, kind 'goal'|'task', title, what NULL, why, owner_id NULL (orphaned),
                 home_parent_id NULL, coverage NULL, clarity NULL, estimate_days NULL,
                 remaining_days NULL, status 'todo'|'doing'|'done', started_at,
                 private BOOL (goals; cascades via home parent), statement_changed_at,
                 created_by, created_at, archived_at NULL)
node_parents    (child_id, parent_id, PRIMARY KEY(child_id, parent_id))   -- "feeds"
dependencies    (node_id, needs_id, PRIMARY KEY(node_id, needs_id))       -- sibling order
risks           (id, node_id, title, impact_min_days, impact_max_days, created_by, created_at)
risk_likelihood (risk_id, likelihood_pct, set_by, set_at)                 -- latest row is current

memberships     (node_id, user_id, role 'contributor'|'watcher', status 'pending'|'active',
                 invited_by, invited_at, joined_at)
transfers       (id, node_id, from_user, to_user, note, reason 'manual'|'handover'|'orphan',
                 status 'pending'|'accepted'|'declined', created_at, resolved_at)

events          (id, node_id, actor_id NULL, type, payload JSON, at)       -- audit + Updates
event_reads     (user_id, last_read_event_id)
sim_runs        (id, root_goal_id, input_hash, iterations, seed, started_at, finished_at)
sim_goal        (run_id, goal_id, p50_days, p85_days, p50_date, p85_date, spread, exposure_days)
sim_node        (run_id, node_id, goal_id, criticality, slack_p50, expected_delay_removed)
```

Rules enforced in `fluxo-core`, inside a transaction, rather than by the database:
- A task never has children.
- `node_parents` stays free of cycles.
- `dependencies` stays free of cycles. A dependency may point at any step the author can see, including
  one under a different goal (shown in that goal's graph as an external "Waiting on" input). **(D16)**
- An item's home parent must be one of its parents.

**Visibility (reading):** every goal and task is visible to every signed-in user, unless it is
private. `nodes.private` marks a goal private; an item is private if it, or any goal on its
**home-parent chain**, is marked private. A private item is visible only to its owner and to people
holding a role on it or on an ancestor. Private steps appear in other goals' graphs as placeholders
("private step · 6d") and still count in forecasts.

**Roles (editing and following):** start at the node and walk up through **every** parent. The most
specific active membership found wins. The owner of the node or of any ancestor counts as an owner.
Contributors edit; Watchers get Updates. **(D17)**

---

## 5. Simulation engine (`fluxo-sim`)

**Inputs** (taken from a snapshot of one top-level goal and everything beneath it):
- **Task durations.** Each task has a likely estimate *L* (its remaining days if in progress).
  Clarity sets the spread as multiples of *L* (min, max):

  | Clarity | Range |
  |---|---|
  | high | 0.85×–1.3× |
  | medium | 0.75×–1.6× |
  | low | 0.6×–2.2× |

  Each run samples from a PERT-beta distribution over that range. Done tasks take 0 days.
  The multipliers need tuning on real data. **(D18)**
- **Risks.** In each run, every risk fires with its likelihood. When it fires, it adds a uniform sample
  from its impact range to its node.
- **Coverage gap.** A goal with partial or low coverage gets a hidden "unknown work" step that its known
  work feeds into:

  | Coverage | Unknown work, as a share of the known critical length |
  |---|---|
  | partial | 0–25% |
  | low | 0–75% |

  When a goal is "low", its dates are reported as **nebulous** rather than as calendar dates. **(D19)**
- **Shared nodes.** A node that feeds several goals is sampled **once per run**, so a slip in shared
  work hits every goal it feeds in the same run. All goals that share descendants are simulated together.

**Per run:**
1. Work through each goal's children in dependency order. A child starts when everything it needs has
   finished. A sub-goal's duration is its own result from the same run.
2. A goal finishes when its last child (or its unknown work) finishes.
3. Record which nodes lie on that run's longest path. When paths tie, credit each tied node.

**Outputs:**
- Criticality % per node within each goal.
- P50 and P85 finish in working days, converted to dates with the goal owner's calendar. **(D20)**
- Slack at P50, and risk exposure (the sum of likelihood × average impact).

**Focus ranking** ("expected delay removed"):
- Recommended start: an approximation. For tasks, criticality × (expected duration − likely duration).
  For risks, criticality × likelihood × average impact. For gaps, the expected size of the unknown work.
- Then re-check the top 20 candidates exactly: re-run with that item resolved, using the same random
  numbers, so the difference reflects only that change. **(D21)**

**Targets:**
- 10,000 runs for a goal tree of 500 nodes in under 1 s on one VM core with rayon. This is the riskiest
  technical assumption, so it gets a spike in Phase 0.
- Results are stored per run, with a hash of the inputs, so the forecast is skipped when nothing changed.

---

## 6. Frontend

- **Templates mirror the mock pages one to one.** The mocks become the reference: `styles.css`,
  `theme.js` and `app.js` carry over, and their markup is split into askama templates (a shared base
  with the top bar, plus one template per page).
- **`app.js` is split into native ES modules** (graph, risks, otp, focus, theme, forms). There is no
  build step, no framework and no bundler. Modules load only on pages that need them.
- **Inline edits** (risk likelihood slider, status, estimate) save through `fetch` to small JSON
  endpoints. The same change also works as a plain form submit when JavaScript is off. "Changes save
  automatically" applies only to task and risk fields. Goal statements are saved explicitly, through
  Goal settings.
- **Accessibility:** keyboard reachable, visible focus, and a text alternative to the graph (the
  ordered list the mocks already fall back to). Axe checks run in CI.
- **Content-Security-Policy:** `default-src 'self'`, with no inline script. The mocks' few inline
  handlers (such as the Copy buttons) move into modules. **(D22)**

---

## 7. Phases (front-loading risk)

Each phase ends with something usable and demonstrable. The riskiest unknowns come first.

| Phase | Scope | Exit criteria |
|---|---|---|
| **0. Spikes** | (a) Engine prototype on a generated DAG of 500 nodes with shared nodes and risks. (b) An askama + axum page served without JS. (c) A TOTP enrolment round trip. | Engine meets the 1 s target or we know the fallback (fewer runs, incremental runs); the stack choices D3–D7 are confirmed in code. |
| **1. Foundations** | Workspace, migrations, config, instance setup (first Administrator), sign-up/in/out, sessions, 2FA, personal recovery codes, Settings (account, password, 2FA, theme, calendar, sessions). | Every auth flow in the mocks works with and without JS. Integration tests cover them. |
| **2. Goals & tasks** | Create, edit and archive goals and tasks; parents with a home parent; dependencies; risks with history; Goals page; goal and sub-goal pages (plaque + list fallback); task page; access rules. | You can model the mock data ("Launch fluxo.dev beta") through the UI. Property tests cover the graph rules. |
| **3. Forecasts** | Simulation worker, stored results, forecast band, distribution chart, the path graph with criticality, coverage gap step, nebulous goals. | The goal page matches the mock using real numbers. Performance is checked against the target. |
| **4. Collaboration** | Invitations, roles and inheritance, ownership transfers, the events log, Updates on Focus, the unread badge. | Two people can share a goal end to end, including accept and decline. |
| **5. Focus** | Ranking (approximation + exact re-check), Do / De-risk / Clarify tabs, snooze and mark started. | The Focus list ranks sensibly on dogfood data. |
| **6. Administrator** | Administrator recovery codes (issue, use, revoke, expiry), the orphaned items queue, promote/demote with the last-Administrator guard, account deletion with editable handover. | Every Administrator flow in the mocks works (Settings → Administrator). The handover rules are covered by property tests. |
| **7. Launch** | Deployment (D12), backups (D13), rate limiting on sign-in and code entry, security headers, error pages, an export of your own data **(D23)**, fluxo.dev live. | We plan fluxo's own next goals in fluxo. |

**Phase 1 decisions (2026-11-16):**
- Sign-up mode is an Administrator setting: open, closed or invite-only, **default invite-only**.
- Only Administrators create sign-up invitations: single-use links (`/join/…`) valid for 7 days, listed and revocable in Settings.
- Usernames: 3–32 characters of `a–z 0–9 . - _`, starting and ending with a letter or digit, no two separators in a row, misleading names reserved.
- When sign-up isn't open, the sign-in page hides "Create an account".
- Theme preference is stored in the account and rendered into each page.
- Sign-in throttling stays in Phase 7.

**Phase 2 decisions (2026-11-16):**
- Only the owner (of the goal or a goal above it) changes a goal's statement, why, coverage and privacy; statement changes are recorded. Contributors change the path beneath it.
- A goal is finished when its owner marks it achieved; fluxo suggests it once every step is done. Achieved goals leave forecasts but stay visible.
- Removal archives (the owner's action, restorable). Permanent deletion is an Administrator action, from Settings → Administrator → Archived goals and tasks.
- Steps are added with an inline form on the goal page; full details live on each step's own page.
- A task may be attached to no goal (owners can remove its last goal). Its page shows a notice recommending it be attached to the goal it serves or removed, and the Goals page lists such tasks. Principle: model people's real work and avoid rules they'd work around (jobs to be done).

**Phase 3 decisions (2026-11-16):**
- Dates use the jiff library (time zones bundled).
- Forecasts are recalculated when a page needs one and its inputs' fingerprint has changed; there is no background worker.
- The highlighted "most likely critical path" is every step that is critical in at least 50% of runs.
- Forecasts count from the owner's next working day; tasks in progress use their remaining days.

Dogfooding: from Phase 3 on, this plan's phases are entered into fluxo itself as the "Launch fluxo.dev
beta" goal.

---

## 8. Product decisions (decided 2026-11-16)

- **D16** Dependencies may cross goals. A dependency from outside a goal is drawn as an external "Waiting on" input, and the simulation includes it.
- **D17** Roles flow down from **every** parent; the most specific role wins.
- **D18** Clarity spreads (multiples of the likely estimate): high 0.85–1.3, medium 0.75–1.6, low 0.6–2.2.
- **D19** Unknown work scales with the known critical path: partial 0–25%, low 0–75%. Low-coverage goals show "nebulous" dates.
- **D20** Dates use the goal owner's calendar.
- **D21** Focus ranks by a fast estimate, then re-simulates the top 20 exactly (same random numbers).
- **D22** Strict CSP: no inline scripts.
- **D23** People can export, as JSON, everything they can see.
- **D24** A declined handover goes to the Administrators' list of items without an owner.
- **Invitations** expire after 14 days.
- **Updates** reach people only inside fluxo (Focus). No email or push in the first release.
- **Visibility:** goals are public to signed-in users by default. Owners can mark a goal private, which cascades down. A shared step follows its home-parent chain. Search shows only what you can see. (Details in section 4.)

**Mock follow-ups** these decisions create (to fold into the templates in Phases 2–4):
- A "Private" toggle in Goal settings, and a lock marker on private goals.
- A public goal you have no role on is read-only, with a "Watch" button so you can follow its Updates.
- External "Waiting on" inputs in the path graph (D16).

---

## 9. Risks to this plan (fluxo's own risk register)

| Risk | Likelihood | Impact | Response |
|---|---|---|---|
| The engine is too slow for big goal trees | 35% | +5–10d | Phase 0 spike first. Fallbacks: adapt the run count, re-simulate only stale goals. |
| Shared nodes make the scheduling semantics confusing | 30% | +3–6d | Property tests, and an explanation of shared steps on the node page. |
| Ranking feels arbitrary to users | 40% | +3–8d | Show "why" on every Focus item (as in the mocks), and tune on dogfood data. |
| Doing everything without JS doubles the work | 25% | +4–8d | Inline edits are always also a form, so each action has one handler and two ways in. |
| Account recovery with no email causes support load | 20% | +1–3d | Administrator recovery codes (already designed). Document the flow. |
