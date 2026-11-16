-- One-time recovery codes that an Administrator gives to someone who is locked out.
-- Codes carry ~59 bits of randomness, work once and expire after 24 hours, so a SHA-256 hash is
-- enough to store them.

CREATE TABLE admin_codes (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    issued_by   INTEGER NOT NULL REFERENCES users(id),
    code_hash   BLOB    NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    used_at     INTEGER,
    revoked_at  INTEGER
) STRICT;
CREATE INDEX admin_codes_by_user ON admin_codes(user_id);

-- Set after signing in with an Administrator's code: every page leads to "Set a new password"
-- until it's done.
ALTER TABLE users ADD COLUMN must_reset_password INTEGER NOT NULL DEFAULT 0;
