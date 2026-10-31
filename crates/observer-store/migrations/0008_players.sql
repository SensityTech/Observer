-- Players as Observer sees them: one row per licence, with the profile's lifetime counters, play
-- time per day, and every identifier and name the licence was seen with. Times are RFC 3339.

CREATE TABLE player (
    id           INTEGER PRIMARY KEY,
    license      TEXT    NOT NULL UNIQUE CHECK (license GLOB 'license:*'),
    -- 16 random bytes: what the metrics database's sessions carry instead of the licence.
    pseudonym    BLOB    NOT NULL UNIQUE CHECK (length(pseudonym) = 16),
    -- The latest.
    name         TEXT    NOT NULL CHECK (length(CAST(name AS BLOB)) <= 256),
    first_seen   TEXT    NOT NULL,
    last_seen    TEXT    NOT NULL,
    -- Time connected that Observer saw.
    play_ms      INTEGER NOT NULL DEFAULT 0 CHECK (play_ms >= 0),
    -- From txAdmin's players database, kept apart.
    imported_ms  INTEGER NOT NULL DEFAULT 0 CHECK (imported_ms >= 0),
    sessions     INTEGER NOT NULL DEFAULT 0 CHECK (sessions >= 0),
    longest_ms   INTEGER NOT NULL DEFAULT 0 CHECK (longest_ms >= 0),
    crashes      INTEGER NOT NULL DEFAULT 0 CHECK (crashes >= 0),
    timeouts     INTEGER NOT NULL DEFAULT 0 CHECK (timeouts >= 0),
    kicks        INTEGER NOT NULL DEFAULT 0 CHECK (kicks >= 0),
    imported_at  TEXT
) STRICT;

-- Play time per day, a date in the schedule's time zone.
CREATE TABLE player_day (
    player       INTEGER NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    day          TEXT    NOT NULL,
    play_ms      INTEGER NOT NULL CHECK (play_ms >= 0),
    sessions     INTEGER NOT NULL CHECK (sessions >= 0),
    PRIMARY KEY (player, day)
) STRICT, WITHOUT ROWID;

-- A confirmed session not credited to its profile yet: deleted in the transaction that credits
-- it. The metrics session carries the same key.
CREATE TABLE player_open_session (
    key          BLOB    PRIMARY KEY CHECK (length(key) = 16),
    player       INTEGER NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    started      TEXT    NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX player_day_by_day ON player_day (day, player, play_ms);
CREATE INDEX player_by_play ON player ((play_ms + imported_ms), id);
CREATE INDEX player_by_first_seen ON player (first_seen, id);
CREATE INDEX player_by_last_seen ON player (last_seen, id);
CREATE INDEX player_by_sessions ON player (sessions, id);
CREATE INDEX player_by_name ON player (name COLLATE NOCASE, id);

-- Every other identifier a player showed: license2, steam, discord, fivem, xbl, live, a hardware
-- token, or the keyed fingerprint of an address. Stored once, whoever showed it.
CREATE TABLE identifier (
    id           INTEGER PRIMARY KEY,
    kind         TEXT    NOT NULL CHECK (kind IN ('license2', 'steam', 'discord', 'fivem', 'xbl', 'live',
                                                  'token', 'ip')),
    -- Lowercase, without the `kind:` prefix; a token keeps its type; an address is the hex of its
    -- keyed fingerprint.
    value        TEXT    NOT NULL CHECK (length(value) <= 128),
    UNIQUE (kind, value)
) STRICT;

-- An observation: this licence connected showing this identifier. Never a merge.
CREATE TABLE player_identifier (
    player       INTEGER NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    identifier   INTEGER NOT NULL REFERENCES identifier (id),
    first_seen   TEXT    NOT NULL,
    last_seen    TEXT    NOT NULL,
    times        INTEGER NOT NULL CHECK (times >= 1),
    -- 'imported': from txAdmin and not seen by Observer since, never a strong link.
    source       TEXT    NOT NULL DEFAULT 'seen' CHECK (source IN ('seen', 'imported')),
    PRIMARY KEY (player, identifier)
) STRICT, WITHOUT ROWID;

CREATE TABLE player_name (
    player       INTEGER NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL CHECK (length(CAST(name AS BLOB)) <= 256),
    first_seen   TEXT    NOT NULL,
    last_seen    TEXT    NOT NULL,
    PRIMARY KEY (player, name)
) STRICT, WITHOUT ROWID;

CREATE INDEX player_identifier_by_identifier ON player_identifier (identifier, player);
CREATE INDEX player_identifier_last_seen ON player_identifier (last_seen);
CREATE INDEX player_name_last_seen ON player_name (last_seen);

-- Licences forgotten on request, as keyed fingerprints. Also kept in a file outside the backups,
-- merged back at every start and before every import.
CREATE TABLE player_forgotten (
    fingerprint  BLOB    PRIMARY KEY CHECK (length(fingerprint) = 16),
    at           TEXT    NOT NULL
) STRICT, WITHOUT ROWID;
