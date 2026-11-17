-- How long archived goals and tasks are kept before they're deleted for good (Administrators set it).
ALTER TABLE instance ADD COLUMN archive_retention_days INTEGER NOT NULL DEFAULT 30;
