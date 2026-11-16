-- Goals and tasks left without an owner (when an account is deleted and no one is suggested, or
-- a suggested owner declines): who had it, and since when. Administrators assign them.
ALTER TABLE nodes ADD COLUMN previous_owner_id INTEGER REFERENCES users(id);
ALTER TABLE nodes ADD COLUMN unowned_since INTEGER;
