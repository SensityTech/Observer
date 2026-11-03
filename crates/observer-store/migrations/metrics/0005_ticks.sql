-- svMain's tick histogram per slot, minute and hour, from which the frame time's percentiles are
-- read; and the links between a crash wave and a change that someone said are not related.

-- A set of FXServer's bounds, in milliseconds, as a JSON array: builds before mid-2024 have others.
CREATE TABLE tick_bounds (
    id     INTEGER PRIMARY KEY,
    bounds TEXT    NOT NULL UNIQUE CHECK (json_valid(bounds))
) STRICT;

-- The ticks of each interval of the bounds: 0 is under the first bound, the last beyond the last
-- bound. Intervals without ticks have no row.
CREATE TABLE tick_5s (
    ts     INTEGER NOT NULL,
    bounds INTEGER NOT NULL REFERENCES tick_bounds (id),
    bucket INTEGER NOT NULL CHECK (bucket >= 0),
    ticks  INTEGER NOT NULL CHECK (ticks > 0),
    PRIMARY KEY (ts, bounds, bucket)
) STRICT, WITHOUT ROWID;

CREATE TABLE tick_1m (
    ts     INTEGER NOT NULL,
    bounds INTEGER NOT NULL REFERENCES tick_bounds (id),
    bucket INTEGER NOT NULL CHECK (bucket >= 0),
    ticks  INTEGER NOT NULL CHECK (ticks > 0),
    PRIMARY KEY (ts, bounds, bucket)
) STRICT, WITHOUT ROWID;

CREATE TABLE tick_1h (
    ts     INTEGER NOT NULL,
    bounds INTEGER NOT NULL REFERENCES tick_bounds (id),
    bucket INTEGER NOT NULL CHECK (bucket >= 0),
    ticks  INTEGER NOT NULL CHECK (ticks > 0),
    PRIMARY KEY (ts, bounds, bucket)
) STRICT, WITHOUT ROWID;

-- A link the timeline drew, which someone said is not related: by its key, who and when.
CREATE TABLE link_dismissed (
    link  TEXT    PRIMARY KEY CHECK (length(CAST(link AS BLOB)) <= 256),
    by    TEXT    NOT NULL,
    at_ms INTEGER NOT NULL
) STRICT;
