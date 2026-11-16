-- A rough size for a goal that hasn't been broken down yet ("probably 5–15 working days").
-- Used by forecasts only while the goal has no steps; optional.
ALTER TABLE nodes ADD COLUMN size_min_days REAL;
ALTER TABLE nodes ADD COLUMN size_max_days REAL;
