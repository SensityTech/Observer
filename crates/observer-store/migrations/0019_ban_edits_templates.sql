-- What the staff changed on a ban since it was made: its reason, its end, or both, each with what
-- it replaced. The ban holds the latest; a row here is never changed. Times are RFC 3339.
CREATE TABLE ban_edit (
    id             INTEGER PRIMARY KEY,
    ban            INTEGER NOT NULL REFERENCES ban (id) ON DELETE CASCADE,
    at             TEXT    NOT NULL,
    by_user        INTEGER,
    by_name        TEXT    NOT NULL,
    reason_before  TEXT    NOT NULL CHECK (length(reason_before) <= 200),
    reason_after   TEXT    NOT NULL CHECK (length(reason_after) <= 200),
    -- NULL for a permanent ban.
    ends_before    TEXT,
    ends_after     TEXT,
    CHECK (reason_before <> reason_after OR ends_before IS NOT ends_after)
) STRICT;

CREATE INDEX ban_edit_by_ban ON ban_edit (ban, id);

-- The reasons and lengths the ban dialog offers, in the order the settings list them.
CREATE TABLE ban_template (
    position     INTEGER PRIMARY KEY,
    reason       TEXT    NOT NULL CHECK (length(reason) BETWEEN 1 AND 200),
    -- NULL for a permanent ban; ten years at most.
    minutes      INTEGER CHECK (minutes BETWEEN 1 AND 5256000)
) STRICT;
