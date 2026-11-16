-- Instance settings, sign-up invitations, personal recovery codes, and per-account preferences.

CREATE TABLE instance (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    signup_mode         TEXT    NOT NULL DEFAULT 'invite' CHECK (signup_mode IN ('open', 'invite', 'closed')),
    setup_completed_at  INTEGER
) STRICT;
INSERT INTO instance (id) VALUES (1);

-- Single-use sign-up links created by Administrators. Only the SHA-256 of the token is stored.
CREATE TABLE signup_invites (
    id          INTEGER PRIMARY KEY,
    token_hash  BLOB    NOT NULL UNIQUE,
    created_by  INTEGER NOT NULL REFERENCES users(id),
    note        TEXT    NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    used_by     INTEGER REFERENCES users(id),
    used_at     INTEGER,
    revoked_at  INTEGER
) STRICT;

-- Personal recovery codes, shown once when 2FA is turned on. Stored as argon2id hashes.
CREATE TABLE recovery_codes (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    used_at     INTEGER
) STRICT;
CREATE INDEX recovery_codes_by_user ON recovery_codes(user_id);

ALTER TABLE users ADD COLUMN theme TEXT NOT NULL DEFAULT 'system' CHECK (theme IN ('system', 'light', 'dark'));
-- A secret waiting for its first code during 2FA setup; replaces totp_secret_enc once confirmed.
ALTER TABLE users ADD COLUMN totp_pending_enc BLOB;
ALTER TABLE users ADD COLUMN totp_enabled_at INTEGER;

-- Turns working days into dates for goals this person owns (D20).
CREATE TABLE user_calendar (
    user_id        INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    workdays_mask  INTEGER NOT NULL DEFAULT 31,      -- bit 0 = Monday … bit 6 = Sunday; 31 = Mon–Fri
    time_zone      TEXT    NOT NULL DEFAULT 'UTC',   -- IANA name
    days_off       TEXT    NOT NULL DEFAULT '[]'     -- JSON array of YYYY-MM-DD
) STRICT;
