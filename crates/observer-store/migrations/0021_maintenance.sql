-- Maintenance mode: the players who still get in while the server is closed. Whether it is closed,
-- since when and by whom live in `settings`, with the owner's message. Times are RFC 3339.

CREATE TABLE maintenance_pass (
    id        INTEGER PRIMARY KEY,
    license   TEXT    NOT NULL UNIQUE CHECK (license GLOB 'license:*'),
    -- Who it is, as the owner wrote it beside the licence.
    note      TEXT    CHECK (length(note) <= 64),
    by_user   INTEGER,
    by_name   TEXT    NOT NULL,
    added_at  TEXT    NOT NULL
) STRICT;
