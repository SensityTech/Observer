-- The processes FXServer's tree started: one row per run, program and command line, a repeat
-- counted. A run is told apart by when it started and its generation.
CREATE TABLE tree_process (
    id              INTEGER PRIMARY KEY,
    run_started     TEXT    NOT NULL,
    generation      INTEGER NOT NULL,
    pid             INTEGER NOT NULL,
    parent_pid      INTEGER,
    parent_image    TEXT,
    -- NULL when the process ended before it could be read.
    image           TEXT,
    -- Masked, at most 4 KiB.
    command_line    TEXT    CHECK (command_line IS NULL OR length(CAST(command_line AS BLOB)) <= 4096),
    truncated       INTEGER NOT NULL CHECK (truncated IN (0, 1)),
    kind            TEXT    NOT NULL CHECK (kind IN ('dumpServer', 'consoleHost', 'node', 'shell', 'other')),
    started_at      TEXT    NOT NULL,
    last_started_at TEXT    NOT NULL,
    ended_at        TEXT,
    count           INTEGER NOT NULL CHECK (count >= 1)
) STRICT;

CREATE UNIQUE INDEX tree_process_program
    ON tree_process (run_started, generation, coalesce(image, ''), coalesce(command_line, ''));
CREATE INDEX tree_process_started ON tree_process (started_at);

-- A crash, a hang Observer killed, or dumps found with neither. The files stay listed once the
-- dump policy or the owner removed them, with `kept` false.
CREATE TABLE crash (
    id             INTEGER PRIMARY KEY,
    at             TEXT    NOT NULL,
    kind           TEXT    NOT NULL CHECK (kind IN ('crash', 'hang', 'found')),
    run_started    TEXT,
    generation     INTEGER,
    build          INTEGER,
    exit           TEXT,
    -- `0xC0000005`.
    exit_code      TEXT,
    signal         INTEGER,
    hang_cause     TEXT    CHECK (hang_cause IS NULL OR hang_cause IN ('noHeartbeat', 'noHttp')),
    signature      TEXT    CHECK (signature IS NULL OR length(CAST(signature AS BLOB)) <= 256),
    analysis_state TEXT    NOT NULL CHECK (analysis_state IN ('pending', 'done', 'noDump', 'unreadable')),
    analysis       TEXT    CHECK (analysis IS NULL OR json_valid(analysis)),
    watchdog_stack TEXT    CHECK (watchdog_stack IS NULL OR length(CAST(watchdog_stack AS BLOB)) <= 4096),
    -- [{ "name", "bytes", "kind", "kept" }]
    files          TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(files) AND json_type(files) = 'array'),
    updated_at     TEXT    NOT NULL
) STRICT;

CREATE INDEX crash_at ON crash (at);
CREATE INDEX crash_signature ON crash (signature, at);

-- Where the tree's processes connected, by remote address and port and the program holding the
-- connections. Written when one is new, its peak grows, or it is seen a minute after the last write.
CREATE TABLE connection_seen (
    protocol   TEXT    NOT NULL CHECK (protocol IN ('tcp', 'udp')),
    address    TEXT    NOT NULL,
    port       INTEGER NOT NULL CHECK (port BETWEEN 0 AND 65535),
    process    TEXT    NOT NULL,
    first_seen TEXT    NOT NULL,
    last_seen  TEXT    NOT NULL,
    peak       INTEGER NOT NULL CHECK (peak >= 0),
    PRIMARY KEY (protocol, address, port, process)
) STRICT, WITHOUT ROWID;

CREATE INDEX connection_seen_last ON connection_seen (last_seen);
