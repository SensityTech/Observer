-- Players' sessions, from the bridge's join and drop reports. Times are Unix milliseconds, as
-- `server_run` has them.

-- The resources that kicked a player, by name; never deleted, so that ids are never reused.
CREATE TABLE resource_name (
    id   INTEGER PRIMARY KEY,
    name TEXT    NOT NULL UNIQUE CHECK (length(CAST(name AS BLOB)) <= 256)
) STRICT;

-- One row per player session of a run. Open while `ended_ms` is NULL.
CREATE TABLE player_session (
    id              INTEGER PRIMARY KEY,
    run             INTEGER NOT NULL REFERENCES server_run (id),
    net_id          INTEGER NOT NULL,
    -- When the join was seen; NULL when the session started before Observer saw it.
    joined_ms       INTEGER,
    -- When Observer first saw the session: its join, or the roster or drop that showed it.
    seen_ms         INTEGER NOT NULL,
    ended_ms        INTEGER,
    -- On FXServer's game timer for a reported drop, on the wall clock for a session Observer
    -- closed itself.
    length_ms       INTEGER CHECK (length_ms IS NULL OR length_ms >= 0),
    category        TEXT    CHECK (category IN ('quit', 'kick', 'timeout', 'crash', 'security',
                                   'serverStop', 'serverCrash', 'unknown')),
    kind            TEXT,
    native_category INTEGER CHECK (native_category BETWEEN 1 AND 64),
    resource        INTEGER REFERENCES resource_name (id),
    -- Redacted: a crash's text, or a kick's or an unknown drop's reason.
    crash_text      TEXT    CHECK (crash_text IS NULL OR length(CAST(crash_text AS BLOB)) <= 512),
    reason_text     TEXT    CHECK (reason_text IS NULL OR length(CAST(reason_text AS BLOB)) <= 512),
    phase           TEXT    CHECK (phase IN ('loading', 'inGame')),
    -- The player's pseudonym, set once their identity is confirmed.
    player          BLOB    CHECK (player IS NULL OR length(player) = 16),
    -- The key of the crediting record in the state database.
    credit          BLOB    CHECK (credit IS NULL OR length(credit) = 16),
    -- Closed by Observer rather than reported.
    synthesized     INTEGER NOT NULL DEFAULT 0 CHECK (synthesized IN (0, 1)),
    CHECK ((ended_ms IS NULL) = (category IS NULL))
) STRICT;

-- Net ids wrap within a long run: only an open session is unique per id.
CREATE UNIQUE INDEX player_session_open ON player_session (run, net_id) WHERE ended_ms IS NULL;
-- Covers the crash rate over the sessions kept.
CREATE INDEX player_session_ended ON player_session (ended_ms, category, length_ms);
CREATE INDEX player_session_category ON player_session (category, ended_ms);
CREATE INDEX player_session_resource ON player_session (resource, ended_ms) WHERE resource IS NOT NULL;
CREATE INDEX player_session_player ON player_session (player, ended_ms) WHERE player IS NOT NULL;
CREATE INDEX player_session_run ON player_session (run);

-- When FXServer's process started, which is how the state database's crashes know the run.
ALTER TABLE server_run ADD COLUMN spawned_ms INTEGER;
