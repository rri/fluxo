-- Ownership transfers, read state for Updates, and the last forecast reported for each goal
-- (to notice when it moves).

CREATE TABLE transfers (
    id           INTEGER PRIMARY KEY,
    node_id      INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    from_user    INTEGER REFERENCES users(id),
    to_user      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    note         TEXT    NOT NULL DEFAULT '',
    created_at   INTEGER NOT NULL,
    status       TEXT    NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'declined', 'cancelled')),
    resolved_at  INTEGER
) STRICT;
CREATE INDEX transfers_by_recipient ON transfers(to_user, status);

-- Everything up to this event id has been seen on Focus.
CREATE TABLE update_reads (
    user_id            INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    last_read_event_id INTEGER NOT NULL DEFAULT 0
) STRICT;

-- The P50/P85 (working days) last reported for a goal; a move of a working day or more is an Update.
CREATE TABLE forecast_marks (
    goal_id   INTEGER PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    p50_days  REAL NOT NULL,
    p85_days  REAL NOT NULL,
    at        INTEGER NOT NULL
) STRICT;

CREATE INDEX events_by_time ON events(at);
