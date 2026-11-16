-- Accounts and sign-in sessions. Times are Unix seconds (UTC).

CREATE TABLE users (
    id                INTEGER PRIMARY KEY,
    username          TEXT    NOT NULL UNIQUE,
    display_name      TEXT    NOT NULL DEFAULT '',
    password_hash     TEXT    NOT NULL,            -- argon2id PHC string (D6)
    totp_secret_enc   BLOB,                        -- XChaCha20-Poly1305: nonce || ciphertext (D10)
    totp_enabled      INTEGER NOT NULL DEFAULT 0,
    totp_last_step    INTEGER,                     -- last accepted 30 s step; blocks code replay
    is_administrator  INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    deleted_at        INTEGER
) STRICT;

CREATE TABLE sessions (
    id_hash          BLOB    PRIMARY KEY,          -- SHA-256 of the cookie value; the value itself is never stored
    user_id          INTEGER REFERENCES users(id) ON DELETE CASCADE,
    stage            TEXT    NOT NULL CHECK (stage IN ('anonymous', 'second_factor', 'signed_in')),
    csrf_token       TEXT    NOT NULL,             -- D9
    failed_attempts  INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL,
    last_seen_at     INTEGER NOT NULL,
    user_agent       TEXT    NOT NULL DEFAULT ''
) STRICT;

CREATE INDEX sessions_by_user ON sessions(user_id);
