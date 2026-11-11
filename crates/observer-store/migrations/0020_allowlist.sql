-- The allowlist: the licences the staff approved, and what refused players asked while it is on.
-- Times are RFC 3339.

CREATE TABLE allowlist_approval (
    id           INTEGER PRIMARY KEY,
    license      TEXT    NOT NULL UNIQUE CHECK (license GLOB 'license:*'),
    -- The name the request carried, shown until the player's profile gives one.
    name         TEXT    CHECK (length(CAST(name AS BLOB)) <= 256),
    -- Who it is, as the staff wrote it when approving ahead of a first visit.
    note         TEXT    CHECK (length(note) <= 64),
    -- The code of the request it answered.
    request      TEXT,
    by_user      INTEGER,
    by_name      TEXT    NOT NULL,
    approved_at  TEXT    NOT NULL
) STRICT;

-- One request per licence: a player who tries again keeps their code.
CREATE TABLE allowlist_request (
    id           INTEGER PRIMARY KEY,
    code         TEXT    NOT NULL UNIQUE CHECK (length(code) = 7),
    license      TEXT    NOT NULL UNIQUE CHECK (license GLOB 'license:*'),
    name         TEXT    CHECK (length(CAST(name AS BLOB)) <= 256),
    first_at     TEXT    NOT NULL,
    last_at      TEXT    NOT NULL,
    tries        INTEGER NOT NULL DEFAULT 1 CHECK (tries >= 1)
) STRICT;

CREATE INDEX allowlist_request_by_last ON allowlist_request (last_at, id);
