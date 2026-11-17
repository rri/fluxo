# fluxo threat model

Written 2026-11-16 for the launch on fluxo.dev and re-assessed on 2026-11-17 after a penetration
test (§7). It covers what's worth protecting, who might attack
it and how, what stops each attack, and what's knowingly left open. Tests for these are in
`crates/fluxo-web/tests/security.rs` and `spec.rs` (rule IDs refer to `docs/spec.md`).

## 1. What's worth protecting

| Asset | Why it matters |
|---|---|
| Private goals and everything homed under them | Plans people chose not to share: titles, whys, steps, risks, people, history. |
| Accounts | An account can change or archive everything its owner owns, and see their private work. |
| Administrator accounts | Can issue recovery codes (take over any account), change sign-up, make Administrators, delete archived work for good. |
| Integrity of plans and forecasts | People decide what to work on from Focus and forecasts. Quiet edits mislead them. |
| `secret.key` | Decrypts every authenticator secret. |
| The database and its backups | All of the above, at rest (nightly snapshots and pre-deploy copies on the server). |
| Availability | One small server, shared with other sites. |
| The deploy pipeline | Whatever CI ships runs on the server as `fluxo`. |

## 2. Who might attack

1. **Anyone on the internet**: no account; fluxo.dev is public and its source is open (AGPL).
2. **A signed-in member**: wants to see private work, take over work they don't own, or slow
   fluxo down for everyone.
3. **A Contributor or Watcher on someone's goal**: has some legitimate access and wants more.
4. **Someone holding a stolen password, stolen session cookie, or unlocked laptop.**
5. **A malicious or compromised Administrator.** Trusted by design; see accepted risks.
6. **Supply chain**: a compromised crate, GitHub Action or CI secret.
7. **Someone with access to the server or a backup.**

## 3. Trust boundaries and entry points

```
Browser ──HTTPS──▶ Caddy (TLS, :443) ──HTTP 127.0.0.1:8371──▶ fluxo ──▶ SQLite (/mnt/data/fluxo)
                                                                 └──▶ secret.key (outside the database)
GitHub Actions ──Tailscale (tag:ci)──▶ ssh deploy@server ──sudo──▶ fluxo-deploy (only)
```

Entry points: about 60 routes (all of them in `crates/fluxo-web/src/lib.rs`), all as form
POSTs or GETs. Also the server log (it holds the setup link), the deploy SSH key and the CI
secrets.

## 4. Threats, and what stops them

Status: **Mitigated** (it was already in place), **Fixed** (fixed in this review),
**Accepted** (left open on purpose, with the reason).

### Spoofing: getting in as someone else

| # | Threat | Status | How |
|---|---|---|---|
| S1 | Guessing passwords online | Mitigated | argon2id; refused after 10 failures per username or 30 per address in 15 minutes (V4); usernames normalised so case changes don't reset the count; the same timing for unknown usernames. |
| S2 | Guessing authenticator codes by restarting sign-in (5 tries each, unlimited restarts) | **Fixed** | Wrong codes now count toward the account's and the address's sign-in limits, and a half-signed-in attempt stops once the limit is reached. About 10 guesses per 15 minutes, against a one-in-a-million code. |
| S3 | Guessing someone's recovery codes or an Administrator's recovery code | **Fixed** / Mitigated | Recovery-code guesses now count the same way. Administrator codes: 12 characters from 31 (≈59 bits), 24 hours, already counted. |
| S4 | A stolen session guessing the password in Settings, to escalate to a password change or account deletion | **Fixed** | Password confirmations (change password, new recovery codes, turn off 2FA, delete account) count toward the same limits and run off the async threads. |
| S5 | IPv6 attackers rotating addresses within their /64 | **Fixed** | Addresses are counted per /64 for IPv6. |
| S6 | Session fixation or a leaked session ID | Mitigated | New random 256-bit ID at each sign-in step (V6). Only its SHA-256 is stored. HttpOnly, Secure, SameSite=Lax. Idle limits of 1 hour, 10 minutes and 30 days. "Sign out everywhere else"; a password change signs out other devices. |
| S7 | Claiming Administrator on a fresh public server before its owner does | Mitigated | `/setup` needs the one-time token printed only in the server log (F2, F3). |
| S8 | Reusing a TOTP code | Mitigated | Each 30-second step is claimed atomically once. |
| S9 | Faking the client address with `X-Forwarded-For` to dodge limits | Mitigated | The header is used only when the connection comes from a trusted proxy (127.0.0.1 = Caddy), and only its last entry. |

### Tampering: changing what you shouldn't

| # | Threat | Status | How |
|---|---|---|---|
| T1 | Cross-site request forgery | Mitigated | Every POST checks the session's form token in constant time (G3; tested on every route). SameSite=Lax. `form-action 'self'`. |
| T2 | **A Contributor making themselves an owner by linking someone's item under a goal they own**, since owners above own everything below; or making private work public by moving its home | **Fixed** | A non-owner's link changes (add, change home, remove) are refused when they'd change who owns the item or whether it's private (L2). See decision 1. |
| T3 | Raising your own role, or acting on items you can't see | Mitigated | Every item action loads access from the database and checks the needed level; Administrator actions check the role on the server (S3). |
| T4 | Two Administrators removing each other at once, leaving none | **Fixed** | Removal is now a single conditional update. Deleting your account re-checks inside its transaction. |
| T5 | Leftover ownership offers after an account is deleted (two pending offers on one item) | **Fixed** | The handover cancels the person's pending offers and any on the items it hands over (D3). |
| T6 | Joining an archived item through an old invitation | **Fixed** | It can't be accepted while the item is archived (I1). |
| T7 | SQL injection | Mitigated | Every query is a compile-time-checked `sqlx` query with bound parameters. |
| T8 | Loops in links that break forecasts | Mitigated | Checked with the simulation engine's own validation. |
| T9 | Titles that display as something else: invisible bidirectional overrides (U+202E…) or control characters pasted into a title, name or note | **Fixed** | Removed from all typed text before saving; single-line fields also lose line breaks (G13). |
| T10 | A malformed parent ID on "New goal" silently making a top-level goal | **Fixed** | Refused with 404, and nothing is created (GO6). Not exploitable, but a silent wrong result. |
| T11 | Mass assignment or unexpected fields (`owner_id`, `role=owner`, extra form fields) | Mitigated | Forms deserialize into fixed structs; unknown fields are ignored and owner/role changes go through their own checked actions (tested). |

### Repudiation: denying what you did

| # | Threat | Status | How |
|---|---|---|---|
| R1 | "I didn't change that goal" | Mitigated | Every change to goals and tasks is an event with the actor and time, shown in History and exported. |
| R2 | Administrator actions leaving no trace | **Fixed** | Role changes, recovery codes issued and used, offers of unowned work, deletes for good and account deletions are now all logged in the server journal. |

### Information disclosure

| # | Threat | Status | How |
|---|---|---|---|
| I1 | Seeing a private item by address | Mitigated | Missing and private items get the same 404 (G4). |
| I2 | **Titles of private goals leaking through labels**: goal pickers ("Private › Sub-goal"), Focus ("On the critical path of …", the goals pills, "blocked by"), Updates ("In …"), "Needs: …" on a goal's path, and the goals an offered item feeds | **Fixed** | Every place that names another item now uses the title only if the person can read it, else "a private goal" / "a private step" without a link (FO5, GP4, T1). |
| I3 | Signed-out visitors learning whether a risk exists | **Fixed** | `/risks/<id>` checks sign-in first (V1). |
| I4 | Cross-site scripting | Mitigated | askama escapes everything. The one raw value is the server-drawn chart, built from numbers (G10, tested with hostile input everywhere). Strict CSP with no inline script or style. |
| I5 | Clickjacking | Mitigated | `frame-ancestors 'none'`, plus `X-Frame-Options: DENY` (**added**). |
| I6 | Downgrade to HTTP, or cookie theft over HTTP | Mitigated / **Fixed** | `.dev` is HSTS-preloaded; Caddy redirects to HTTPS; Secure cookies; HSTS header **added**. |
| I7 | Export or backups exposing others' private work | Mitigated | Export holds only what the person can read. Backups stay on the server's volume (mode 600, owned by `fluxo`). |
| I8 | Reading TOTP secrets from a stolen database or backup | Mitigated | Sealed with XChaCha20-Poly1305 under `secret.key`, which isn't in the database or the backups; each is bound to its user. |
| I9 | Server details in errors | Mitigated | Generic 500 page; details only in the journal. Deploy output in the public CI log prints only the deploy script's own steps. |
| I10 | Username enumeration | Accepted | Sign-up says a username is taken, and People invitations say "No one has that username". People need these to find each other. Sign-in itself doesn't reveal it. |
| I11 | Watchers seeing an item's title after it's archived ("X archived") | Accepted | They could already see it; the update tells them where it went. |
| I12 | Deploy details in the public repository: the server's Tailscale address and its SSH host key | Accepted | The address is reachable only inside the tailnet, and a host key is public by nature. Neither grants access; the deploy key and Tailscale credentials are only in GitHub's encrypted secrets. |
| I13 | Secrets committed to the repository | Mitigated | Checked across every commit and branch: no private keys, tokens, passwords or `.env` files. `secret.key`, the database and backups live only on the server; `.gitignore` covers build output and test artifacts. |
| I14 | Invitation links and recovery codes held between a post and the page after it | Mitigated | Kept in process memory only (never the database or logs), keyed by the session that made them, taken on first read and dropped after 5 minutes or a restart. Only that Administrator's next Settings page shows them. |
| I15 | Archived work kept indefinitely | Mitigated | Archived items are deleted for good after the retention period an Administrator sets (7–365 days). Backups still hold them for up to 7 nights. |

### Denial of service

| # | Threat | Status | How |
|---|---|---|---|
| D1 | Filling the database with anonymous sessions (each visit to /login makes one) | **Fixed** | Idle sessions and old sign-in failures are purged every 15 minutes (SE1). |
| D2 | Tying up the server with argon2 (about 20–50 ms each) | **Fixed** / Mitigated | Password checks now run on the blocking pool, not the async threads, so other requests keep flowing. Failures are limited per username and address. |
| D3 | Huge form bodies | Mitigated | axum's 2 MB limit; titles ≤200, whys ≤2,000, task texts ≤5,000, notes ≤500. |
| D4 | A member creating tens of thousands of items, so every forecast (10,000 runs over the whole instance) slows down for everyone | Accepted for now | See decision 3. Forecasts run on the blocking pool and are cached by fingerprint; 500 items take about 45 ms. |
| D5 | Locking someone out by failing sign-in with their username | Accepted | The price of S1: 15 minutes at a time; an Administrator's recovery code still works. |
| D6 | Open sign-up used to create many accounts | Accepted | Invitation-only by default; open sign-up is an Administrator's choice and can be closed at any time. |
| D8 | **One member nesting goals thousands deep, making every person's pages slow**: access checks walked everything above each item, so cost grew with depth squared, on the request threads (2,000 levels: 0.5 s per page for everyone; 20,000: about 50 s) | **Fixed** | Access is worked out once per request in one pass over the tree (linear in items and links), and checked against the item-by-item rules on random graphs (PF1). Other people's pages went from 0.49 s to 0.12 s at 2,000 levels. |
| D9 | Goal pickers growing with depth squared ("A › B › … › Z" for every goal): an 18 MB "New goal" page at 2,000 levels | **Fixed** | Labels show at most the two goals above (G14): 126 KB at the same depth. |
| D7 | Losing the server or its data volume | Accepted | Backups are on the same volume, with no off-site copy (decided 2026-11-16). Copy a snapshot off the server by hand when it matters. |

### Elevation of privilege

| # | Threat | Status | How |
|---|---|---|---|
| E1 | Member → Administrator | Mitigated | Only Administrators change roles (tested). |
| E2 | Contributor → owner | **Fixed** | See T2. |
| E3 | Administrator taking over any account with a recovery code | Accepted (by design) | That is the recovery mechanism. It's now logged; the person gets their 2FA turned off and must set a new password, so it can't be done silently. |
| E4 | Code execution on the server from a deploy | Mitigated | The deploy key can run only `fluxo-deploy`. It checks the checksum, runs the new binary as `fluxo` before installing it, and rolls back if it's unhealthy. CI runs only on `main` in the `production` environment; actions are pinned to commits; `cargo audit` runs on every build. fluxo runs as an unprivileged user under systemd hardening. |
| E6 | A workflow on another branch using the deploy secrets | Mitigated / **Action** | The deploy job runs only for `main`. Restricting the `production` environment to `main` in GitHub makes the secrets unavailable to any other branch as well (see `deploy/README.md`). |
| E5 | Supply-chain compromise of a crate | Partly mitigated | `Cargo.lock`, `--locked`, `cargo audit`, few dependencies. One advisory is waived (RUSTSEC-2026-0459, in SQLite; sqlx doesn't allow the fixed version yet). |

## 5. Server and operations

- **secret.key**: mode 600, owned by `fluxo`. Copy it off the server once; without it 2FA must be
  set up again (recovery is still possible).
- **The setup link** is in the journal until setup is done. Anyone who can read the journal
  (`adm` or `systemd-journal` group, root) could claim the instance first, so do setup right
  after the first deploy.
- **Backups**: snapshots and pre-deploy copies are mode 600 in `/mnt/data/fluxo/backups`, on
  the same volume as the database. There is no off-site copy (decided 2026-11-16): losing the volume
  or the server loses the data, up to whatever is copied off by hand (see D7).
- **Shared server**: fluxo runs as its own user, separate from the other sites on the server. fluxo's unit can write only to its data
  directory, has no capabilities, and is limited to `@system-service` system calls and IP and
  Unix sockets (**added**; tested locally under the same settings).

## 6. Decisions (confirmed 2026-11-16)

1. **Relinking (T2)**: Contributors keep the freedom to link work wherever it serves, except
   where that would change who owns it or who can see it. Those changes need an owner.
   Alternatives: only owners can change links at all (simpler, more friction), or allow it and
   accept the escalation.
2. **One failure counter (S2–S4)**: wrong codes and wrong confirmation passwords share sign-in's
   limit of 10 per username in 15 minutes. That makes the counter simple and closes every guessing
   path. The cost is that a burst of wrong codes also pauses password sign-in for that account.
3. **Item volume (D4)**: no cap for now. If it's ever needed, a generous per-person creation rate
   (for example 500 items an hour) would stop abuse without affecting real use.

## 7. Penetration test (2026-11-17)

Run against a local release build with three accounts (an Administrator, a Contributor on a
shared goal, a member with no roles), a private goal with a task and a risk, and demo data.

| Area | What was tried | Result |
|---|---|---|
| Access control | The two other accounts, against the private goal, its task and risk: every GET, and all 28 kinds of POST (steps, statement, coverage, achieved, size, task edits, start, every link and need direction, risks, archive/restore/delete, people, transfers, invitations, snooze, every Administrator action) | All refused (404/403/redirect); the database was unchanged except harmless no-ops. No private title on any page or in the export. |
| CSRF | Missing token, another session's token, foreign `Origin` | Refused (403/422), nothing changed. |
| Session | Reuse after sign-out; IDs at each sign-in step | Old sessions dead (also V6, SE2 tests). |
| Input | 46 hostile forms: 100 KB titles, NUL/control/bidi characters, non-numeric and overflowing IDs, `NaN`, `inf`, `1e308`, negatives, inverted ranges, unknown choices, path-like time zones, 5,000-line day-off lists, `role=owner`, self-transfers | No server errors; numbers bounded; **found T9, T10** (fixed). |
| Transport and parsing | 3 MB body, duplicate fields, JSON, invalid encoding; PUT/DELETE/PATCH/TRACE/OPTIONS | 413, 422, 415, 422; 405. |
| Static files | `..`, `%2e%2e`, `%2f`, NUL, case tricks | Only the five named files are served. |
| Headers | Every kind of response (page, 404, static, health, export) | CSP, nosniff, frame and referrer headers everywhere; `no-store` on pages; no server banner. |
| Host header | Forged `Host` for invitation links and redirects | Production sets `public_url`, so links never use the request's host; redirects are relative. |
| Enumeration | Sign-in timing, known vs unknown username | 20 ms vs 20 ms. Random invitation tokens: "This link can't be used". |
| Denial of service | 2,000 nested goals; then pages for the owner and for others | **Found D8, D9** (fixed). |
| Races | 20 parallel acceptances of one ownership transfer | Exactly one took effect. |
