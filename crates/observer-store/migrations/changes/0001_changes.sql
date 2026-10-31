-- What each start of FXServer ran from, apart from Observer's state: set aside and started again
-- when it cannot be read, and never in the daily backup. Instants are UTC text, as in the state
-- database. File digests are keyed (HMAC-SHA-256 under an instance key); the plain SHA-256 of a
-- binary, encrypted or generated file is kept beside its digest in the tree's listing.

-- A start that was scanned, or the latest check of the disk asked for in the panel.
CREATE TABLE snapshot (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id      TEXT    NOT NULL,
    kind           TEXT    NOT NULL CHECK (kind IN ('start', 'check')),
    -- The run as the state database names it (tree_process, crash): its start and generation.
    run_started    TEXT,
    generation     INTEGER,
    start_cause    TEXT    CHECK (start_cause IN ('requested', 'restart', 'scheduled', 'recovery')),
    started_by     TEXT,
    build          INTEGER CHECK (build BETWEEN 1000 AND 9999999),
    build_sha256   TEXT,
    game           TEXT    CHECK (game IN ('gta5', 'rdr3')),
    game_build     INTEGER,
    -- OneSync, auto start, txAdmin events, the cfg file, extra argument names with value tags; no
    -- argument value.
    launch         TEXT    NOT NULL CHECK (json_valid(launch)),
    -- The files of the cfg exec chain with their digests, and the resources the chain starts.
    cfg            TEXT    NOT NULL CHECK (json_valid(cfg)),
    -- The resources the bridge reported started after `ready`; NULL until it did.
    started        TEXT    CHECK (started IS NULL OR json_valid(started)),
    -- The complete snapshot holding the resource rows when they all equal its own; never a chain.
    same_as        INTEGER REFERENCES snapshot (id) ON DELETE RESTRICT,
    baseline       INTEGER NOT NULL CHECK (baseline IN (0, 1)),
    state          TEXT    NOT NULL CHECK (state IN ('listing', 'hashing', 'complete', 'partial', 'failed')),
    partial_reason TEXT    CHECK (partial_reason IN ('limit', 'cancelled', 'diskFloor', 'unreadable')),
    -- Why a failed snapshot has no listing, in English, for the log and the panel's detail.
    failure        TEXT,
    files          INTEGER,
    bytes          INTEGER,
    taken_at       TEXT    NOT NULL,
    listed_at      TEXT,
    hashed_at      TEXT,
    UNIQUE (server_id, run_started, generation)
) STRICT;

CREATE INDEX snapshot_taken ON snapshot (server_id, kind, taken_at);
CREATE INDEX snapshot_same_as ON snapshot (same_as) WHERE same_as IS NOT NULL;

CREATE TABLE snapshot_resource (
    snapshot_id INTEGER NOT NULL REFERENCES snapshot (id) ON DELETE CASCADE,
    -- `resources/[qb]/qb-core`.
    path        TEXT    NOT NULL,
    -- In lower case, as FXServer and the cfg compare them; shown_name keeps the folder's case.
    name        TEXT    NOT NULL,
    shown_name  TEXT    NOT NULL,
    -- Another folder of the snapshot has the same name.
    duplicate   INTEGER NOT NULL CHECK (duplicate IN (0, 1)),
    -- NULL until every entry of the resource is hashed.
    tree        TEXT    REFERENCES tree (sha256),
    -- While some entries wait for the hashing pass: deflate of the entries known so far, those
    -- waiting with an empty key, and how many wait.
    pending_listing BLOB,
    pending     INTEGER NOT NULL DEFAULT 0,
    version     TEXT,
    -- The manifest's facts as JSON.
    manifest    TEXT    CHECK (manifest IS NULL OR json_valid(manifest)),
    escrowed    INTEGER NOT NULL CHECK (escrowed IN (0, 1)),
    -- { "branch", "commit" }
    git_head    TEXT    CHECK (git_head IS NULL OR json_valid(git_head)),
    -- [{ "name", "version" }] of node_modules.
    packages    TEXT    CHECK (packages IS NULL OR json_valid(packages)),
    files       INTEGER NOT NULL,
    bytes       INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, path)
) STRICT, WITHOUT ROWID;

CREATE INDEX snapshot_resource_name ON snapshot_resource (snapshot_id, name);
CREATE INDEX snapshot_resource_tree ON snapshot_resource (tree) WHERE tree IS NOT NULL;

-- A resource's entries, once however many snapshots share them.
CREATE TABLE tree (
    sha256  TEXT PRIMARY KEY,
    -- deflate of the sorted entries as JSON: path, size, key, kind and the plain SHA-256 shown.
    listing BLOB NOT NULL
) STRICT, WITHOUT ROWID;

-- The text entries of each tree, so that SQL can tell which starts use which text version.
CREATE TABLE tree_text (
    tree   TEXT NOT NULL REFERENCES tree (sha256) ON DELETE CASCADE,
    path   TEXT NOT NULL,
    digest TEXT NOT NULL,
    PRIMARY KEY (tree, path)
) STRICT, WITHOUT ROWID;

CREATE INDEX tree_text_digest ON tree_text (digest);

-- The texts a snapshot holds outside its trees: the files of the cfg exec chain.
CREATE TABLE snapshot_text (
    snapshot_id INTEGER NOT NULL REFERENCES snapshot (id) ON DELETE CASCADE,
    path        TEXT    NOT NULL,
    digest      TEXT    NOT NULL,
    PRIMARY KEY (snapshot_id, path)
) STRICT, WITHOUT ROWID;

CREATE INDEX snapshot_text_digest ON snapshot_text (digest);

-- What holds for one snapshot only.
CREATE TABLE snapshot_flag (
    snapshot_id INTEGER NOT NULL REFERENCES snapshot (id) ON DELETE CASCADE,
    path        TEXT    NOT NULL,
    flag        TEXT    NOT NULL CHECK (flag IN ('afterStart', 'unsettled', 'reloadedLive')),
    PRIMARY KEY (snapshot_id, path, flag)
) STRICT, WITHOUT ROWID;

-- Text as it may be shown, masked before it is stored, under the keyed digest of the bytes on disk.
CREATE TABLE text_version (
    digest     TEXT    PRIMARY KEY,
    -- deflate of the masked UTF-8 text; NULL once the budget or a purge dropped it.
    body       BLOB,
    -- The stored body's size, compressed, which the budget counts.
    stored     INTEGER NOT NULL DEFAULT 0,
    bytes      INTEGER NOT NULL,
    lines      INTEGER NOT NULL,
    -- The version of the masking rules the body was masked with.
    masking    INTEGER NOT NULL,
    -- Where each redaction lies in the masked text and its tag, six bytes each.
    redactions BLOB    NOT NULL,
    -- Snapshot ids: the first and the newest snapshot whose trees or texts list it. They may name a
    -- snapshot retention removed.
    first_seen INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX text_version_seen ON text_version (last_seen) WHERE body IS NOT NULL;
CREATE INDEX text_version_masking ON text_version (masking) WHERE body IS NOT NULL;

-- What the last scan saw of each file, so that only what changed is read again. A cache: lost, the
-- next scan hashes everything.
CREATE TABLE file_index (
    path      TEXT    PRIMARY KEY,
    size      INTEGER NOT NULL,
    mtime_ns  INTEGER NOT NULL,
    -- Linux only: inode and change time, which no copy tool sets back.
    inode     INTEGER,
    ctime_ns  INTEGER,
    digest    TEXT,
    -- The plain SHA-256 of a binary, encrypted or generated file.
    sha256    TEXT,
    kind      TEXT    NOT NULL,
    -- Hashed less than two seconds after its last write: read again next time.
    racy      INTEGER NOT NULL DEFAULT 0 CHECK (racy IN (0, 1)),
    hashed_at TEXT
) STRICT, WITHOUT ROWID;

-- Resources started, stopped or restarted while a run was up.
CREATE TABLE live_change (
    id          INTEGER PRIMARY KEY,
    snapshot_id INTEGER NOT NULL REFERENCES snapshot (id) ON DELETE CASCADE,
    at          TEXT    NOT NULL,
    name        TEXT    NOT NULL,
    change      TEXT    NOT NULL CHECK (change IN ('started', 'stopped', 'restarted')),
    -- Who typed the matching command in Observer's console, and the command, masked; NULL when it
    -- was made another way.
    by          TEXT,
    command     TEXT
) STRICT;

CREATE INDEX live_change_snapshot ON live_change (snapshot_id, at);
