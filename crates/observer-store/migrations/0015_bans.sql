-- Bans: who the server refuses, with every identifier and hardware token the ban holds. A ban is
-- never deleted: lifted, it stays with who lifted it and why. Times are RFC 3339.

CREATE TABLE ban (
    id           INTEGER PRIMARY KEY,
    code         TEXT    NOT NULL UNIQUE CHECK (length(code) = 9),
    -- The profile the ban was made from; forgetting the player keeps the ban.
    player       INTEGER REFERENCES player (id) ON DELETE SET NULL,
    name         TEXT    CHECK (length(CAST(name AS BLOB)) <= 256),
    license      TEXT    CHECK (license GLOB 'license:*'),
    reason       TEXT    NOT NULL CHECK (length(reason) <= 200),
    by_user      INTEGER,
    by_name      TEXT    NOT NULL,
    created_at   TEXT    NOT NULL,
    -- NULL for a permanent ban.
    ends_at      TEXT,
    lifted_at    TEXT,
    lifted_by    TEXT,
    lift_reason  TEXT    CHECK (length(lift_reason) <= 200),
    -- Connections refused and players dropped.
    refused      INTEGER NOT NULL DEFAULT 0 CHECK (refused >= 0),
    CHECK ((lifted_at IS NULL) = (lifted_by IS NULL))
) STRICT;

-- What the ban refuses: the licence and the other identifiers in the players' canonical form, and
-- hardware tokens with their type.
CREATE TABLE ban_identifier (
    ban          INTEGER NOT NULL REFERENCES ban (id) ON DELETE CASCADE,
    kind         TEXT    NOT NULL CHECK (kind IN ('license', 'license2', 'steam', 'discord', 'fivem', 'xbl',
                                                  'live', 'token')),
    value        TEXT    NOT NULL CHECK (length(value) <= 128),
    PRIMARY KEY (ban, kind, value)
) STRICT, WITHOUT ROWID;

CREATE INDEX ban_identifier_by_value ON ban_identifier (kind, value, ban);
CREATE INDEX ban_by_player ON ban (player);
