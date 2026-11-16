-- Failed sign-in attempts (password or Administrator recovery code), for slowing down guessing.
-- Rows older than a day are pruned as new ones arrive.
CREATE TABLE sign_in_failures (
    id        INTEGER PRIMARY KEY,
    username  TEXT    NOT NULL,
    ip        TEXT    NOT NULL,
    at        INTEGER NOT NULL
) STRICT;
CREATE INDEX sign_in_failures_by_username ON sign_in_failures(username, at);
CREATE INDEX sign_in_failures_by_ip ON sign_in_failures(ip, at);
