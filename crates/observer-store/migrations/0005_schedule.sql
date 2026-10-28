-- FXServer's restart schedule, one row per server. A missing row means the defaults: off.
CREATE TABLE schedule_settings (
    server_id        TEXT    PRIMARY KEY,
    enabled          INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    mode             TEXT    NOT NULL CHECK (mode IN ('fixed_times', 'interval')),
    -- ["04:00", ...], ascending.
    times            TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(times) AND json_type(times) = 'array' AND json_array_length(times) <= 48),
    interval_minutes INTEGER NOT NULL CHECK (interval_minutes BETWEEN 30 AND 10080),
    -- An IANA name; NULL follows the host's zone.
    time_zone        TEXT    CHECK (time_zone IS NULL OR length(time_zone) BETWEEN 1 AND 64),
    -- Minutes before each restart, descending.
    warnings         TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(warnings) AND json_type(warnings) = 'array' AND json_array_length(warnings) <= 16),
    skip_after_start INTEGER NOT NULL CHECK (skip_after_start BETWEEN 0 AND 720),
    version          INTEGER NOT NULL DEFAULT 1,
    updated_at       TEXT    NOT NULL
) STRICT, WITHOUT ROWID;

-- What the schedule keeps between two runs of Observer: a skipped occurrence, a delayed one, a
-- single restart someone planned, and the latest occurrence dealt with, which a clock set back
-- never replays. Instants are UTC text, as everywhere else.
CREATE TABLE schedule_plan (
    server_id        TEXT PRIMARY KEY,
    skip_at          TEXT,
    skip_by          TEXT,
    delay_occurrence TEXT,
    delay_to         TEXT,
    delay_by         TEXT,
    one_off_at       TEXT,
    one_off_by       TEXT,
    settled_at       TEXT,
    updated_at       TEXT NOT NULL,
    CHECK ((skip_at IS NULL) = (skip_by IS NULL)),
    CHECK ((delay_occurrence IS NULL) = (delay_to IS NULL) AND (delay_to IS NULL) = (delay_by IS NULL)),
    CHECK (delay_to IS NULL OR delay_to > delay_occurrence),
    CHECK ((one_off_at IS NULL) = (one_off_by IS NULL))
) STRICT, WITHOUT ROWID;
