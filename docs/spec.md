# fluxo: intended behaviour

How every page and action should behave, across the states a visitor can be in. Each rule has
an ID; the tests that check it name it (`crates/fluxo-web/tests/spec.rs`,
`crates/fluxo-web/tests/security.rs`, `tests/browser/`); `docs/spec-coverage.md` maps every rule
to its tests. Where this document and the code disagree, the code is wrong unless a decision in
`docs/implementation-plan.md` says otherwise.

## 0. States and roles

| State | Meaning |
|---|---|
| **Fresh** | No Administrator yet (setup not done). |
| **Visitor** | Setup done; not signed in (no session, or an anonymous one). |
| **Half-signed-in** | Password accepted, authenticator or recovery code still owed (10-minute limit). |
| **Must reset** | Signed in with an Administrator's recovery code; owes a new password. |
| **Member** | Signed in, not an Administrator. |
| **Administrator** | Signed in, with the Administrator role. |

On a goal or task, a signed-in person has one access level: **None** (can't see it),
**Read** (public item, no role), **Watch**, **Contribute** or **Own**. Roles flow down from
every parent goal, nearest wins (equally near: Contributor beats Watcher); owning anything above
an item makes you an owner of it. An item
is private if it or any goal on its home-parent chain is marked private. Archived items are
visible only to their owners.

Content states that change what pages show: no goals; goals without steps; steps without
forecasts (nebulous or unsized); work shared with others; private work; archived work;
unattached tasks; pending invitations and transfers; unread updates.

## 1. Everywhere

- **G1** Every response carries a strict Content-Security-Policy (`default-src 'self'`, no
  inline script or style, `frame-ancestors 'none'`, `form-action 'self'`), `X-Content-Type-Options:
  nosniff`, `Referrer-Policy: same-origin`, `X-Frame-Options: DENY`, `Cross-Origin-Opener-Policy`
  and a restrictive `Permissions-Policy`. Pages (not `/static/`) are `Cache-Control: no-store`.
  Over HTTPS, `Strict-Transport-Security` is set.
- **G2** Every page is complete HTML that works without JavaScript; every action is a form POST
  followed by a redirect or a page. JavaScript only enhances (theme toggle, copy buttons, OTP
  boxes, path graph layout, save-on-change, confirmations, keyboard shortcuts).
- **G3** Every state-changing request needs the session's form token (`csrf`). A missing or wrong
  token gets **403** "This form has expired…" and changes nothing.
- **G4** Unknown addresses get a **404** page (wordmark, "Not found", a link to Focus). A goal or task that doesn't
  exist and one the person can't see get the same **404** ("doesn't exist, or you can't see it"),
  so existence of private items isn't revealed.
- **G5** Server errors get a self-contained **500** page; details go only to the server log.
- **G6** `/api/health` answers `ok` (200) when the database answers, in every state, without
  signing in, and reveals nothing else.
- **G7** Static files: only the ten named files are served (the stylesheet, the two scripts, the
  favicon, the logo, the web app manifest and four PNG icons); anything else under `/static/` is
  404.
- **G8** Every signed-in page has the top bar: wordmark (to Focus), **Focus** (with a count badge
  when updates are unread or something waits on the person), **Goals** (a link, not a menu), the
  theme toggle (shown only with JavaScript), and the account menu (username, Settings, Sign
  out). The page's own nav link is marked `aria-current`. On phone widths (640px and below),
  Focus and Goals leave the top bar and float at the bottom right instead, as two buttons at
  least 56px tall, high enough to clear the home indicator and Safari's toolbar; pages leave room
  below the footer so the buttons never cover their end.
- **G9** The theme is the account's setting (System, Light, Dark), rendered by the server as
  `data-theme` on `<html>`, so there's no flash. Auth pages use System.
- **G10** User-written text (titles, whys, notes, usernames, display names, risk titles) is always
  escaped. The only unescaped markup is the server-drawn forecast chart, built from numbers.
- **G11** Keyboard shortcuts (signed-in pages, not while typing): `g f` Focus, `g g` Goals,
  `g s` Settings.
- **G12** The word "Site Administrator" never appears; the role is "Administrator".
- **G13** Text people type (titles, whys, task texts, risk titles, display names, notes) loses
  control characters and invisible bidirectional overrides before it's saved; single-line fields
  also lose line breaks. Text made only of such characters counts as empty.
- **G14** Goal pickers label each goal with at most the two goals above it ("… › Parent ›
  Goal"), with goals the person can't read as "a private goal".
- **G15** Accessibility: every page has no WCAG 2.1 A/AA violations (axe), in light and dark,
  with and without JavaScript: one `<h1>`, labelled fields, visible focus, sufficient contrast,
  `lang` set, and landmarks.
- **G16** Performance: pages are server-rendered with one stylesheet and two small scripts (one
  deferred), no web fonts or third-party requests, and static files are cached for an hour. The
  service worker (G20) is the only other script, and it never runs in a page.
- **G17** Phone widths: from 320px up, no page scrolls sideways and nothing sticks out of the
  screen. Rows wrap; forms stack; tables become one labelled card per row; the path graph keeps
  steps readable and scrolls sideways inside its card, opening on the goal; long titles wrap
  inside their pills. Centred lines balance instead of leaving a lone word; explanations under
  fields are left-aligned. Hover hints are hidden on touch screens.
- **G18** Every page, signed in or out, including the 404 and 500 pages, ends with the same
  footer: "github.com/rri/fluxo" (a link to the source) · "Source code licensed under AGPL-3.0".
  It sits at the bottom of the screen on short pages, and its two parts stack on phones.
- **G19** Installing: fluxo can be added to a phone's home screen and opens as its own app, at
  Focus, without browser controls. Every page links the web app manifest (name "fluxo", standalone,
  starting at `/`) and a 180px home-screen icon: the white flame on indigo, opaque, as PNG. The
  browser's bar matches the page background in the account's theme. The session cookie lasts 30
  days (the signed-in idle limit) instead of ending with the browser session, and every page that
  loads renews it, so closing the app doesn't sign the person out; the server's idle limits still
  decide when a session ends.
- **G20** Reading offline (with JavaScript): a service worker (`/sw.js`) keeps copies of Focus,
  Goals, and goal and task pages; no other page is ever kept. Pages come from the network first,
  and a copy is saved each time; offline, or when the network takes more than 6 seconds, the saved
  copy is shown. Besides pages the person opened in the last 30 days, it keeps (refreshing at most
  every 30 minutes, at most 400 pages) the person's own: `/offline/pages` lists Focus, Goals and
  every goal and task they own, contribute to or watch, including everything beneath those,
  except archived ones and ones they can't read. A page that's gone or unreadable (404, 403) loses
  its copy. A page without a saved copy shows "Not saved on this device" (`/offline`).
  Offline, and on any saved copy, pages are read-only: a bar says "Offline. Saved <time>. Read
  only." (or, back online, "Saved copy from <time>" with a link to refresh), every button that
  would save is disabled, and what was typed stays in its field. Signing in, signing out, or being
  sent to the sign-in page deletes every saved page, so one person's pages are never shown to
  the next.

## 1a. Robustness

- **PF1** One person's work can't make pages slow for others: what each person can access is
  worked out in one pass per request, so a page's cost grows with the number of items, not with
  how deep goals are nested.
- **PF2** Malformed input never causes a server error: bad numbers, unknown choices, oversized
  bodies (over 2 MB: 413), wrong content types (415) and malformed forms (422) are refused, and
  unknown choices fall back to the default.

## 2. Fresh instance (setup)

- **F1** Every page except `/setup`, `/api/health` and `/static/*` redirects to `/setup`.
- **F2** `/setup` without the one-time token from the server log shows "This fluxo isn't set up
  yet" with where to find the link, and no form.
- **F3** `/setup?token=…` with the right token shows "Set up fluxo": username, password (12+
  characters), confirmation, "Create Administrator account". The form carries the token; a POST
  without it is 403.
- **F4** Creating the account makes it the Administrator, signs it in, marks setup done and goes
  to optional 2FA setup. Only one account can ever be created this way, even if two race.
- **F5** After setup, `/setup` redirects to `/login`.

## 3. Visitor (signed out)

- **V1** Every app page (Focus, Goals, goal and task pages, Settings, export, invitations,
  transfers) redirects to `/login`. App POSTs redirect to `/login` and change nothing. Signed-out
  pages that lead back to `/login` say "Back to sign in".
- **V2** `/login`: "Sign in", username and password, "Locked out?" link, and "Create an
  account" only when sign-up is open.
- **V3** A wrong username or password shows "That username and password don't match." whichever
  was wrong, and takes about as long either way. The username is kept in the field.
- **V4** After 10 failures for a username or 30 from an address (an IPv6 /64 counts as one
  address) within 15 minutes, sign-in is refused with **429** and "Too many unsuccessful
  attempts…", whether or not the password is right. Wrong authenticator codes, recovery codes,
  Administrator recovery codes and passwords typed to confirm Settings actions count toward the
  same limits.
- **V5** Without 2FA, the right password signs in and goes to Focus. With 2FA, it goes to
  `/login/2fa` (Half-signed-in).
- **V6** Every step up (anonymous → half → signed in) issues a new session ID; the old one stops
  working.
- **V7** `/signup`: when sign-up is open, the sign-up form; otherwise "Sign-up isn't open" and
  no form. Usernames: 3–32 of `a–z 0–9 . - _`, case-insensitive, unique, not reserved.
  A taken username shows "That username is taken."
- **V8** `/join/<token>`: a valid, unexpired, unused invitation (and sign-up not closed) shows the
  sign-up form; otherwise "This link can't be used". Each invitation creates exactly one account.
- **V9** `/locked`: username plus an Administrator's recovery code. The right pair signs the
  person in as Must reset, turns their 2FA off, deletes their own recovery codes, and signs out
  every other session. Five wrong tries end the attempt.
- **V10** Signing out (POST with token) ends the session and goes to `/login`.

## 4. Half-signed-in

- **H1** `/login/2fa` asks for the 6-digit code (six boxes with JavaScript, auto-submitting).
  Codes are valid for the current 30-second step ± one, and each works once.
- **H2** `/login/recovery` accepts one of the person's own recovery codes, once each; success goes
  to Settings with "You signed in with a recovery code".
- **H3** Five wrong codes in one attempt end it ("Too many wrong codes. Sign in again…").
- **H4** The half session can't open any app page (they redirect to `/login`), and expires after
  10 minutes idle.

## 5. Must reset

- **R1** Every app page redirects to `/reset-password`, which asks for a new password twice.
- **R2** Saving it clears the requirement and goes to 2FA setup.

## 6. Signed in: Focus (`/`)

- **FO1** Sections in order: "Waiting on you" (only when something waits), "Updates", "Next up".
- **FO2** Waiting on you lists pending, unexpired invitations (to items not archived) and
  ownership transfers, each with "Review".
- **FO3** Updates: outlook-changing events from the last 90 days on items the person owns,
  contributes to or watches (directly or above), newest first, unread ones marked, with "N unread"
  (or "All read") · "last 90 days" beside the heading. Empty state:
  "Nothing yet. Changes to goals…". "Mark all read" appears only when something is
  unread.
- **FO3a** A moved forecast reads "Estimated finish of <goal> moved later/earlier: P85 a → b", with
  P50 beneath. One level down, "Why" lists the recorded changes on the goal, everything beneath it,
  and the work any of that needs, since the last report (newest first, at most 8): steps added,
  archived or restored; status, estimate, days left and clarity; rough size and coverage; risks
  added, raised, lowered or removed; needs and links; achieved and reopened. Titles follow what the
  reader may see. With none, it says time passing moved it. A slipping finish's "Why" is that time
  passed without progress.
- **FO4** Next up: ranked Do / De-risk / Clarify items for work the person can contribute to and
  that feeds a goal, with tabs (All, Do, De-risk, Clarify) and counts. Each item shows why,
  pills, the goals it feeds, "blocked by" when a need isn't done, "Mark started" for startable
  tasks, and "Snooze until tomorrow". Beside each item, the expected days it would remove from
  the goal's finish ("−2.4d"; "~" when estimated rather than rechecked; "<0.1d" when negligible;
  "?" with "not sized" when it can't be measured). At most 30 items. Snoozing is personal and
  lasts until the next day. Empty state on All: "Nothing to rank yet…"; on other tabs:
  "Nothing here right now."
- **FO5** Titles of goals and steps the person can't read never appear (they read "a private
  goal" / "a private step", unlinked).

## 7. Signed in: Goals (`/goals`)

- **GO1** "+ New goal" button. Goals that feed no other goal in fluxo the person can see, theirs first, then ones they
  have a role on, then public ones; each card shows role, private, achieved, why, forecast
  (P50 · P85, or "nebulous"), coverage, steps done, and the step most critical now. Beneath each
  card: the goals that feed it (with their step counts) and how many tasks feed it directly.
- **GO2** No goals: "No goals yet." with a "+ New goal" button.
- **GO3** Tasks attached to no goal that the person can contribute to are listed under "Tasks not
  attached to any goal", with the note that they can't show up on a critical path.
- **GO4** "Archived" (collapsed) lists archived goals that feed no other goal and unattached tasks the person
  owns. Others never see them.
- **GO5** Private goals of others never appear.
- **GO6** A new goal's parent must be a goal the person can contribute to; any other or
  malformed parent is refused (404 or 403) and nothing is created.

## 8. Signed in: a goal (`/goals/<id>`)

- **GP1** Breadcrumbs follow the home chain; goals above that the person can't read show as
  "Private goal" without a link.
- **GP2** Header: Goal, title, why, owner, coverage, Private, Achieved, step count;
  "Edit goal" for owners, "Add a step" for contributors; and when it was set, with "never changed"
  or when its statement last changed.
- **GP3** Forecast band (when there are steps or a rough size): P50, P85, spread, risk exposure,
  run count, when it was updated, the chart, and whose calendar dates use ("your", "ada's", or the
  default Mon–Fri, UTC when no one owns it). Low coverage shows "nebulous" instead of dates.
  "Top risks on the path" lists the risks on steps of the critical path by expected delay.
- **GP4** The path: steps with kind, estimate or size, status and criticality; the most likely
  critical path (≥50% of runs) highlighted. Without JavaScript it's a list with "Needs: …"; with
  JavaScript a layered graph, where hovering or focusing a step traces what it needs and what it
  leads to. Steps the person can't read show as "Private step" without a
  link or title, and their titles don't leak through "Needs" or "Waiting on".
- **GP5** No steps: "No steps yet." plus, for contributors, the rough-size form.
- **GP6** Contributors see "Add a step" (task or goal, estimate/clarity or coverage/size,
  optional "needs"), risk editing and link editing. Watchers and readers see everything
  read-only.
- **GP7** Owners see Goal settings: statement (title, why), coverage, Private, mark achieved /
  reopen (in its own card), Archive (alone in a red-bordered card), People invitations and ownership transfer. "Suggest achieved" appears when
  every step is done.
- **GP8** Archived goals show "This goal is archived and will be permanently deleted on <date>."
  with Restore (owners only; nobody else can open them).
- **GP10** "Archived steps" lists this goal's archived steps with Restore, for their owners.
- **GP11** Goal and task pages end with History (collapsed): the item's 30 latest events, each
  with "Why" when a forecast moved.
- **GP9** People: owner, roles held here, and roles inherited from above (with where they come
  from, or "a private goal"). Owners can invite (Contributor/Watcher), change roles and remove;
  signed-in readers of public items can Watch; people with a direct role can Leave / Stop
  watching.

## 9. Signed in: a task (`/tasks/<id>`)

- **TP1** Breadcrumbs as GP1. Title, status pill, forecast (P50/P85 done, and criticality in each
  goal it feeds).
- **TP2** Definition (what, why), estimate & progress (estimate, clarity, status, remaining when
  in progress): editable for contributors, read-only otherwise.
- **TP3** Connections: what it needs (or "Nothing"), what it unblocks (or "Nothing waits on it"),
  and which goals it feeds, the home one marked (or "No goal yet"). Contributors can add needs (same goal first, then
  elsewhere) and links.
- **TP4** An unattached task shows a notice recommending attaching it to a goal or archiving it,
  with a goal picker for contributors.
- **TP5** Owners see Archive. Archived tasks behave as GP8.

## 10. Links, needs, risks

- **L1** Adding a link (this feeds goal G) needs Contribute on the item and on G, G must be a
  goal and not archived, and it must not create a loop.
- **L2** A Contributor (not an owner) can't make a link change that changes who owns the item or
  whether it's private: adding a link under a goal with different owners, moving its home to a
  goal that changes privacy, or removing the link that makes someone an owner. They get "Only an
  owner can make that change…". Owners can.
- **L3** Removing an item's last link needs Own; the item becomes a goal feeding no other goal in fluxo, or an
  unattached task.
- **L4** Needs: on any item the person can read, not archived, not itself, no loops.
- **L5** Risks: a title (≤200), likelihood 0–100%, impact days min ≤ max (0–1000). 0% retires a
  risk. Likelihood changes are kept as history.

## 11. Archiving and deleting

- **A1** Archiving and restoring are owner actions. Archived items drop out of forecasts and
  Focus and are visible only to their owners, with the date they'll be deleted.
- **A2** No one deletes items by hand. An archived item is deleted for good once it has been
  archived for the retention period (an Administrator picks 7, 30, 90, 180 or 365 days; 30 by
  default), checked hourly. It takes along everything beneath it that feeds nothing else;
  shared steps move their home to a remaining goal. Restoring before then cancels it.

## 12. Settings (`/settings`)

- **S1** Sections for everyone: Account (username, display name ≤64, role), Password (current
  + new twice; changing it signs out other devices), Two-factor authentication (on: since,
  codes left, new codes with password, turn off with password + code; off: set up), Appearance,
  Working calendar (working days, time zone; days off added and removed one date or range at a
  time, shown as ranges, at most a year per range), Sessions (this device marked; sign out
  others), Export, Delete account.
- **S1a** 2FA setup shows a QR code, the key to type ("Can't scan? Enter this key") and the code
  field; "Skip for now" right after joining, "Back to Settings" otherwise. Turning it on, or
  making new codes, shows "Save your recovery codes" once (eight codes, with Copy and Download);
  reloading doesn't show them again.
- **S2** Notices appear in the section they belong to, from a fixed list (never echoed from the
  address; a notice about a person names them by looking up the id in the address).
- **S2a** Every Settings form posts, then redirects (reloading never resubmits). An invitation
  link or recovery code made by a post is held in memory for the next page (5 minutes at most,
  by session), shown once beside what made it, and focused.
- **S3** Members never see the Admin section, and every Administrator action answers them
  403.
- **S4** Administrators also see: sign-up mode (invitation only, open, closed), invitations
  (create with a note; the link is shown once; revoke), recovery codes (issue for someone else,
  shown once, 24 hours; cancel), People (role, 2FA, last active; make or remove Administrator;
  the last Administrator can't be removed), Goals and tasks without an owner (titles only;
  offer to someone), Archived items (retention period; owner, archived and deletion dates).
- **S5** Export downloads JSON of everything the person can read, nothing more.

## 13. Accounts leaving and changing hands

- **D1** Delete account needs the username typed and the password. The only Administrator can't
  delete their account.
- **D2** Each owned item gets a suggested new owner: the nearest owner above, else the
  longest-standing Contributor (on it or above), else the Administrators. The person can change
  each suggestion. Every recipient must accept; declined items go to the Administrators.
  Deleting an account is the only way an item loses its owner: an ordinary transfer that's
  declined leaves the item with its owner.
- **D3** Pending offers made by the person leaving, and pending offers on items they hand over,
  are replaced by the handover (at most one pending transfer per item).
- **D4** The account can never sign in again; its username stays reserved and in history.
- **T1** Ownership transfers: an owner offers an item to someone; nothing changes until they
  accept; the previous owner stays on as a Contributor. Recipients see the item's title, why and
  the goals it feeds (private ones as "a private goal").
- **I1** Invitations last 14 days. The review page names who invited, the role and what it allows
  (and how many items beneath it), with Decline and Accept. Accepting gives the role; declining
  removes the invitation.
  Invitations to archived items can't be accepted while archived.

## 14. Sessions and housekeeping

- **SE1** Sessions idle out: anonymous after an hour, half-signed-in after 10 minutes, signed in
  after 30 days. Idle sessions and day-old sign-in failures are purged every 15 minutes.
- **SE2** Session cookies are HttpOnly, SameSite=Lax, Secure over HTTPS, and only their SHA-256 is
  stored.
