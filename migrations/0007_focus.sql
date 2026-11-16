-- Focus: items someone has snoozed (hidden for them until a date in their calendar), and the P85
-- date last reported for each goal, for the daily "dates slipping" check.

CREATE TABLE focus_snoozes (
    user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item     TEXT    NOT NULL,              -- "task:12", "risk:5", "goal:3"
    until    TEXT    NOT NULL,              -- YYYY-MM-DD, the person's next working day
    PRIMARY KEY (user_id, item)
) STRICT;

CREATE TABLE slip_marks (
    goal_id   INTEGER PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    p85_date  TEXT    NOT NULL,             -- YYYY-MM-DD in the owner's calendar
    at        INTEGER NOT NULL
) STRICT;
