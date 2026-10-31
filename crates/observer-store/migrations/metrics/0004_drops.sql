-- What players' drops are, grouped: crash signatures, kick and unknown reason groups, bursts of
-- drops, and hourly rollups that outlive the sessions. Hours are Unix seconds, as the samples'
-- `ts`; run and session times stay Unix milliseconds.

-- A crash's signature, built from its text by the normaliser whose version it stores. Kept for
-- good: the rollups refer to it.
CREATE TABLE crash_signature (
    id            INTEGER PRIMARY KEY,
    signature     TEXT    NOT NULL UNIQUE CHECK (length(CAST(signature AS BLOB)) <= 256),
    form          TEXT    NOT NULL CHECK (form IN ('hash', 'symbol', 'error', 'load', 'exception', 'text')),
    -- The blamed module without the game build, and that build.
    module        TEXT    CHECK (module IS NULL OR length(CAST(module AS BLOB)) <= 256),
    game_build    INTEGER,
    -- The crash's code name, `kilo-ohio-lemon`.
    label         TEXT    CHECK (label IS NULL OR length(CAST(label AS BLOB)) <= 256),
    -- For a crash while loading an asset: its resource.
    resource      INTEGER REFERENCES resource_name (id),
    normaliser    INTEGER NOT NULL,
    first_seen_ms INTEGER NOT NULL,
    -- The pseudonym of the player whose crash made it: one player makes at most a few new
    -- signatures a day. Blanked when that player is forgotten.
    first_player  BLOB    CHECK (first_player IS NULL OR length(first_player) = 16)
) STRICT;

CREATE INDEX crash_signature_first_player ON crash_signature (first_player, first_seen_ms)
    WHERE first_player IS NOT NULL;

-- A kick's or an unknown drop's reason with its numbers, names and quoted strings taken out. No
-- example is kept here: examples are read from the sessions, which expire and forget.
CREATE TABLE drop_reason (
    id       INTEGER PRIMARY KEY,
    template TEXT    NOT NULL UNIQUE CHECK (length(CAST(template AS BLOB)) <= 640)
) STRICT;

-- At least 5 unexpected drops within 10 seconds, and at least a fifth of the players online.
CREATE TABLE drop_incident (
    id           INTEGER PRIMARY KEY,
    run          INTEGER NOT NULL REFERENCES server_run (id),
    started_ms   INTEGER NOT NULL,
    ended_ms     INTEGER NOT NULL,
    drops        INTEGER NOT NULL CHECK (drops >= 1),
    -- The sessions open when it started.
    players      INTEGER,
    -- The longest hitch of the samples it spans.
    hitch_max_ms INTEGER
) STRICT;

CREATE INDEX drop_incident_started ON drop_incident (started_ms);

-- The players a crash or a hang of FXServer dropped, counted once its run ended: the sessions
-- that counted them expire, the run stays.
ALTER TABLE server_run ADD COLUMN dropped INTEGER;

ALTER TABLE player_session ADD COLUMN signature INTEGER REFERENCES crash_signature (id);
ALTER TABLE player_session ADD COLUMN reason INTEGER REFERENCES drop_reason (id);
ALTER TABLE player_session ADD COLUMN incident INTEGER REFERENCES drop_incident (id);
-- The normaliser that classified this closed session; 0 until then.
ALTER TABLE player_session ADD COLUMN classified INTEGER NOT NULL DEFAULT 0;

CREATE INDEX player_session_signature ON player_session (signature, ended_ms) WHERE signature IS NOT NULL;
CREATE INDEX player_session_reason ON player_session (reason) WHERE reason IS NOT NULL;
CREATE INDEX player_session_incident ON player_session (incident) WHERE incident IS NOT NULL;
CREATE INDEX player_session_unclassified ON player_session (ended_ms) WHERE classified = 0 AND ended_ms IS NOT NULL;
CREATE INDEX player_session_seen ON player_session (seen_ms);

-- Drops per closed hour, from the sessions that ended in it. `kind` is '' when the session has
-- none (a crash); `subject` is the crash's signature or the kicking resource, 0 for none.
CREATE TABLE drop_1h (
    hour           INTEGER NOT NULL,
    category       TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    subject        INTEGER NOT NULL DEFAULT 0,
    -- Drops, then those of them in an incident, and how many incidents those were: an incident
    -- counts once per category, in the group of its first drop of that category.
    drops          INTEGER NOT NULL,
    incident_drops INTEGER NOT NULL,
    incidents      INTEGER NOT NULL,
    -- Of the drops, those whose phase FXServer could tell, and those of a player still loading.
    phased         INTEGER NOT NULL,
    loading        INTEGER NOT NULL,
    PRIMARY KEY (hour, category, kind, subject)
) STRICT, WITHOUT ROWID;

-- Time played per closed hour, from every session that overlapped it, and the sessions that
-- ended in it.
CREATE TABLE exposure_1h (
    hour      INTEGER PRIMARY KEY,
    player_ms INTEGER NOT NULL CHECK (player_ms >= 0),
    sessions  INTEGER NOT NULL CHECK (sessions >= 0)
) STRICT;

-- The hours whose rollups are written. A session closed or reclassified late takes its hours out,
-- so that they are computed again.
CREATE TABLE drop_rolled (hour INTEGER PRIMARY KEY) STRICT, WITHOUT ROWID;
