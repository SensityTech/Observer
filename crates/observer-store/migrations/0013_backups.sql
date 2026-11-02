-- Backups of the game server's database: what the owner set, where the copies go, every run and
-- what became of each copy. Every timestamp is RFC 3339 UTC with three fractional digits; names
-- are stored as observer-domain writes them. Secrets live in tables of their own, which no read of
-- the settings or the destinations selects.

-- One server's settings. A missing row means the defaults: before scheduled restarts, no fixed
-- time, a minute's wait, MariaDB, the connection string of the cfg.
CREATE TABLE backup_settings (
    server_id            TEXT    NOT NULL PRIMARY KEY,
    before_restarts      TEXT    NOT NULL CHECK (before_restarts IN ('off', 'scheduled', 'allRestarts')),
    -- ["HH:MM", ...], ascending, in the restart schedule's zone.
    times                TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(times) AND json_type(times) = 'array'),
    restart_wait_seconds INTEGER NOT NULL CHECK (restart_wait_seconds BETWEEN 0 AND 600),
    engine               TEXT    NOT NULL CHECK (engine IN ('mariaDb', 'mySql')),
    source               TEXT    NOT NULL CHECK (source IN ('cfg', 'manual')),
    -- The connection typed in, kept while the source is the cfg; its password is apart.
    host                 TEXT,
    port                 INTEGER CHECK (port IS NULL OR port BETWEEN 1 AND 65535),
    user_name            TEXT,
    database_name        TEXT,
    client_folder        TEXT,
    version              INTEGER NOT NULL DEFAULT 1,
    updated_at           TEXT    NOT NULL,
    CHECK ((host IS NULL) = (port IS NULL) AND (port IS NULL) = (user_name IS NULL)
        AND (user_name IS NULL) = (database_name IS NULL)),
    CHECK (source = 'cfg' OR host IS NOT NULL)
) STRICT, WITHOUT ROWID;

CREATE TABLE backup_source_secrets (
    server_id TEXT NOT NULL PRIMARY KEY REFERENCES backup_settings (server_id) ON DELETE CASCADE,
    password  TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE backup_destinations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id  TEXT    NOT NULL,
    name       TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    enabled    INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    kind       TEXT    NOT NULL CHECK (kind IN ('local', 'sftp', 's3')),
    -- The kind's fields as observer-domain writes them, its secret aside.
    target     TEXT    NOT NULL CHECK (json_valid(target) AND json_type(target) = 'object'
        AND json_extract(target, '$.kind') = kind),
    -- The public half of the SFTP key Observer made for it, as an authorized_keys line.
    public_key TEXT,
    keep_last  INTEGER CHECK (keep_last IS NULL OR keep_last BETWEEN 1 AND 1000),
    keep_days  INTEGER CHECK (keep_days IS NULL OR keep_days BETWEEN 1 AND 3650),
    version    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    CHECK (keep_last IS NOT NULL OR keep_days IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX backup_destinations_name ON backup_destinations (server_id, name);

-- The SFTP password or private key, or the S3 secret key.
CREATE TABLE backup_destination_secrets (
    destination_id INTEGER NOT NULL PRIMARY KEY REFERENCES backup_destinations (id) ON DELETE CASCADE,
    secret         TEXT    NOT NULL CHECK (length(secret) > 0)
) STRICT;

-- Every run: the dump, the file made of it and why it failed, if it did.
CREATE TABLE backup_runs (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id         TEXT    NOT NULL,
    trigger           TEXT    NOT NULL CHECK (trigger IN ('schedule', 'restart', 'manual')),
    -- Who asked, or made the restart; NULL when Observer ran it by itself.
    actor             TEXT,
    started_at        TEXT    NOT NULL,
    finished_at       TEXT,
    outcome           TEXT    CHECK (outcome IN ('succeeded', 'partial', 'failed', 'cancelled', 'interrupted')),
    engine            TEXT    NOT NULL CHECK (engine IN ('mariaDb', 'mySql')),
    -- Known once the source is read.
    database_name     TEXT,
    host              TEXT,
    -- The dump tool and its version.
    client            TEXT,
    file_name         TEXT,
    sql_bytes         INTEGER CHECK (sql_bytes IS NULL OR sql_bytes >= 0),
    file_bytes        INTEGER CHECK (file_bytes IS NULL OR file_bytes >= 0),
    sql_sha256        TEXT    CHECK (sql_sha256 IS NULL
        OR (length(sql_sha256) = 64 AND sql_sha256 NOT GLOB '*[^0-9a-f]*')),
    file_sha256       TEXT    CHECK (file_sha256 IS NULL
        OR (length(file_sha256) = 64 AND file_sha256 NOT GLOB '*[^0-9a-f]*')),
    -- FXServer started again before the dump ended.
    server_started    INTEGER NOT NULL DEFAULT 0 CHECK (server_started IN (0, 1)),
    -- ["table", ...]: the first tables whose engine has no transactions, and how many there are.
    loose_tables      TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(loose_tables) AND json_type(loose_tables) = 'array'),
    loose_table_count INTEGER NOT NULL DEFAULT 0 CHECK (loose_table_count >= 0),
    error             TEXT,
    error_detail      TEXT,
    CHECK ((finished_at IS NULL) = (outcome IS NULL)),
    CHECK (outcome IS NULL OR outcome IN ('succeeded', 'partial') OR error IS NOT NULL),
    CHECK (outcome NOT IN ('succeeded', 'partial') OR (file_sha256 IS NOT NULL AND file_bytes IS NOT NULL))
) STRICT;

CREATE INDEX backup_runs_server ON backup_runs (server_id, id);
CREATE INDEX backup_runs_unfinished ON backup_runs (id) WHERE finished_at IS NULL;

-- One copy of a run's file at one destination. Observer deletes only what these rows say it wrote.
-- Deleting a destination forgets its copies: the files stay where they are.
CREATE TABLE backup_copies (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id         INTEGER NOT NULL REFERENCES backup_runs (id) ON DELETE CASCADE,
    destination_id INTEGER NOT NULL REFERENCES backup_destinations (id) ON DELETE CASCADE,
    state          TEXT    NOT NULL CHECK (state IN ('uploading', 'stored', 'failed', 'deleted')),
    -- The file's path in the destination's folder, or the object's key.
    location       TEXT    NOT NULL CHECK (length(location) > 0),
    bytes          INTEGER CHECK (bytes IS NULL OR bytes >= 0),
    started_at     TEXT    NOT NULL,
    stored_at      TEXT,
    deleted_at     TEXT,
    delete_reason  TEXT    CHECK (delete_reason IN ('retention', 'manual')),
    error          TEXT,
    error_detail   TEXT,
    UNIQUE (run_id, destination_id),
    CHECK ((state IN ('stored', 'deleted')) = (stored_at IS NOT NULL)),
    CHECK ((state = 'deleted') = (deleted_at IS NOT NULL) AND (deleted_at IS NULL) = (delete_reason IS NULL)),
    CHECK ((state = 'failed') = (error IS NOT NULL))
) STRICT;

CREATE INDEX backup_copies_destination ON backup_copies (destination_id, state, stored_at);
