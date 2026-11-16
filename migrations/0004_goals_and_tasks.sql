-- Goals and tasks ("nodes"), how they feed and wait on each other, their risks, and who can do what.

CREATE TABLE nodes (
    id                    INTEGER PRIMARY KEY,
    kind                  TEXT    NOT NULL CHECK (kind IN ('goal', 'task')),
    title                 TEXT    NOT NULL,
    what                  TEXT    NOT NULL DEFAULT '',         -- tasks: what will be done
    why                   TEXT    NOT NULL DEFAULT '',
    owner_id              INTEGER REFERENCES users(id),        -- NULL: without an owner (Administrators assign)
    home_parent_id        INTEGER REFERENCES nodes(id),        -- one of the goals it feeds; NULL for top-level
    coverage              TEXT    CHECK (coverage IN ('low', 'partial', 'full')),      -- goals
    clarity               TEXT    CHECK (clarity IN ('low', 'medium', 'high')),        -- tasks
    estimate_days         REAL,                                -- tasks: likely duration, working days
    remaining_days        REAL,                                -- tasks in progress
    status                TEXT    NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'doing', 'done')),
    started_at            INTEGER,
    achieved_at           INTEGER,                             -- goals: owner marked it achieved
    private               INTEGER NOT NULL DEFAULT 0,          -- goals; cascades down the home-parent chain
    statement_changed_at  INTEGER NOT NULL,
    created_by            INTEGER NOT NULL REFERENCES users(id),
    created_at            INTEGER NOT NULL,
    archived_at           INTEGER,
    archived_by           INTEGER REFERENCES users(id),
    CHECK ((kind = 'goal' AND coverage IS NOT NULL) OR (kind = 'task' AND clarity IS NOT NULL AND estimate_days IS NOT NULL))
) STRICT;

-- "child feeds parent". A node may feed several goals; nodes.home_parent_id picks its home.
CREATE TABLE node_parents (
    child_id   INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    parent_id  INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (child_id, parent_id)
) STRICT;
CREATE INDEX node_parents_by_parent ON node_parents(parent_id);

-- "node can't start until needs_id finishes". May cross goals (D16).
CREATE TABLE dependencies (
    node_id   INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    needs_id  INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    added_at  INTEGER NOT NULL,
    PRIMARY KEY (node_id, needs_id),
    CHECK (node_id != needs_id)
) STRICT;
CREATE INDEX dependencies_by_needs ON dependencies(needs_id);

CREATE TABLE risks (
    id               INTEGER PRIMARY KEY,
    node_id          INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    title            TEXT    NOT NULL,
    impact_min_days  REAL    NOT NULL CHECK (impact_min_days >= 0),
    impact_max_days  REAL    NOT NULL,
    created_by       INTEGER NOT NULL REFERENCES users(id),
    created_at       INTEGER NOT NULL,
    CHECK (impact_max_days >= impact_min_days)
) STRICT;
CREATE INDEX risks_by_node ON risks(node_id);

-- Every change of likelihood; the latest row is current. 0 retires a risk.
CREATE TABLE risk_likelihood (
    id              INTEGER PRIMARY KEY,
    risk_id         INTEGER NOT NULL REFERENCES risks(id) ON DELETE CASCADE,
    likelihood_pct  INTEGER NOT NULL CHECK (likelihood_pct BETWEEN 0 AND 100),
    set_by          INTEGER NOT NULL REFERENCES users(id),
    set_at          INTEGER NOT NULL
) STRICT;
CREATE INDEX risk_likelihood_by_risk ON risk_likelihood(risk_id, id);

-- Contributor / Watcher roles; they carry down to everything beneath (D17). Invitations arrive in Phase 4.
CREATE TABLE memberships (
    node_id     INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT    NOT NULL CHECK (role IN ('contributor', 'watcher')),
    status      TEXT    NOT NULL DEFAULT 'active' CHECK (status IN ('pending', 'active')),
    invited_by  INTEGER REFERENCES users(id),
    invited_at  INTEGER NOT NULL,
    joined_at   INTEGER,
    PRIMARY KEY (node_id, user_id)
) STRICT;
CREATE INDEX memberships_by_user ON memberships(user_id);

-- What happened to a goal or task: the audit trail now, Updates on Focus later (Phase 4).
CREATE TABLE events (
    id        INTEGER PRIMARY KEY,
    node_id   INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    actor_id  INTEGER REFERENCES users(id),
    kind      TEXT    NOT NULL,
    detail    TEXT    NOT NULL DEFAULT '{}',   -- JSON
    at        INTEGER NOT NULL
) STRICT;
CREATE INDEX events_by_node ON events(node_id, id);
