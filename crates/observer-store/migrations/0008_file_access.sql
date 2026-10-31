-- File access over SFTP: groups of rules, the accesses that hold them, their keys, the uploads in
-- flight, and the server's own host key. Every timestamp is RFC 3339 UTC with three fractional
-- digits.

-- One server's named sets of rules. observer-domain validates the rules before every write, and
-- they always change together with their group, under its version.
CREATE TABLE file_groups (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id   TEXT    NOT NULL,
    name        TEXT    NOT NULL COLLATE NOCASE,
    description TEXT    NOT NULL DEFAULT '',
    rules       TEXT    NOT NULL CHECK (json_valid(rules) AND json_type(rules) = 'array'),
    version     INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    UNIQUE (server_id, name)
) STRICT;

-- An account's access or a service access: exactly one of user_id and service_name.
CREATE TABLE file_accesses (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       INTEGER UNIQUE REFERENCES users (id) ON DELETE CASCADE,
    service_name  TEXT    UNIQUE COLLATE NOCASE,
    enabled       INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    expires_at    TEXT,
    -- CIDR blocks the client address must fall in; empty means anywhere.
    networks      TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(networks) AND json_type(networks) = 'array'),
    -- Bytes per UTC day.
    daily_budget  INTEGER CHECK (daily_budget > 0),
    -- Bytes per second.
    max_rate      INTEGER CHECK (max_rate > 0),
    version       INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT    NOT NULL,
    last_login_at TEXT,
    last_login_ip TEXT,
    CHECK ((user_id IS NULL) <> (service_name IS NULL))
) STRICT;

CREATE TABLE file_access_groups (
    access_id INTEGER NOT NULL REFERENCES file_accesses (id) ON DELETE CASCADE,
    group_id  INTEGER NOT NULL REFERENCES file_groups (id) ON DELETE CASCADE,
    PRIMARY KEY (access_id, group_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX file_access_groups_group ON file_access_groups (group_id);

-- Public keys only, unique within an access: a login finds its key by login, then fingerprint.
CREATE TABLE file_keys (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    access_id    INTEGER NOT NULL REFERENCES file_accesses (id) ON DELETE CASCADE,
    algorithm    TEXT    NOT NULL,
    public_key   BLOB    NOT NULL,
    fingerprint  TEXT    NOT NULL,
    comment      TEXT    NOT NULL DEFAULT '',
    created_at   TEXT    NOT NULL,
    -- Required for a service access's keys, checked by observer-app; optional for an account's.
    expires_at   TEXT,
    last_used_at TEXT,
    last_used_ip TEXT,
    UNIQUE (access_id, fingerprint)
) STRICT;

CREATE INDEX file_keys_fingerprint ON file_keys (fingerprint);

-- Temporary upload files, recorded before each is created and forgotten after its commit or
-- removal. Every start removes the ones still listed, under the root each was made in.
CREATE TABLE file_uploads (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id  TEXT    NOT NULL,
    -- The root's final path when the file was created.
    root       TEXT    NOT NULL,
    folder     TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    UNIQUE (root, folder, name)
) STRICT;

-- Bytes uploaded per access and UTC day, for the daily budget; past days trimmed by the sweep.
CREATE TABLE file_budget (
    access_id INTEGER NOT NULL REFERENCES file_accesses (id) ON DELETE CASCADE,
    day       TEXT    NOT NULL,
    bytes     INTEGER NOT NULL CHECK (bytes >= 0),
    PRIMARY KEY (access_id, day)
) STRICT, WITHOUT ROWID;

-- The SFTP server's own key. Whoever reads this table can pass for the server, as whoever reads
-- the data folder can read everything else in it.
CREATE TABLE file_host_key (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    algorithm   TEXT    NOT NULL,
    private_key BLOB    NOT NULL,
    fingerprint TEXT    NOT NULL,
    created_at  TEXT    NOT NULL
) STRICT;

-- Failed SFTP logins per login and client network, apart from the panel's so that neither locks
-- or slows the other. The same shape as login_failures, read by the same lock rules.
CREATE TABLE file_login_failures (
    username          TEXT    NOT NULL COLLATE NOCASE,
    network           TEXT    NOT NULL,
    failures          INTEGER NOT NULL,
    window_started_at TEXT    NOT NULL,
    locked_until      TEXT,
    lockouts          INTEGER NOT NULL DEFAULT 0,
    unaudited         INTEGER NOT NULL DEFAULT 0,
    unaudited_since   TEXT,
    last_failure_at   TEXT    NOT NULL,
    PRIMARY KEY (username, network)
) STRICT, WITHOUT ROWID;

-- The Files settings: the shared upload cap in bytes per second (0 for none), the free space an
-- upload must leave on the volume, the largest file any rule allows, and the limit a new rule
-- starts with. `files.version` is bumped by every save.
INSERT INTO settings (key, value) VALUES
    ('files.max_rate', '50000000'),
    ('files.disk_floor', '2147483648'),
    ('files.max_file_size', '2147483648'),
    ('files.default_file_size', '536870912'),
    ('files.version', '1');

-- A pipeline's entries name it as a `service` actor. SQLite cannot change a CHECK, so the audit
-- log is rebuilt: every row copied with its id, then its indexes and triggers made again.
CREATE TABLE audit_log_rebuilt (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         TEXT    NOT NULL,
    actor_kind TEXT    NOT NULL CHECK (actor_kind IN ('user', 'local', 'cli', 'system', 'anonymous', 'service')),
    actor_id   INTEGER,
    actor_name TEXT,
    ip         TEXT,
    user_agent TEXT    CHECK (length(CAST(user_agent AS BLOB)) <= 256),
    action     TEXT    NOT NULL,
    target     TEXT,
    outcome    TEXT    NOT NULL CHECK (outcome IN ('success', 'failure', 'denied')),
    details    TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(details)),
    expires_at TEXT    NOT NULL
) STRICT;

INSERT INTO audit_log_rebuilt (id, at, actor_kind, actor_id, actor_name, ip, user_agent, action, target, outcome, details, expires_at)
SELECT id, at, actor_kind, actor_id, actor_name, ip, user_agent, action, target, outcome, details, expires_at
FROM audit_log ORDER BY id;

-- Copying sets the sequence only to the highest id copied; the highest ever used, purged ones
-- included, is kept so that no id comes back, which keyset pagination relies on.
DELETE FROM sqlite_sequence WHERE name = 'audit_log_rebuilt';
INSERT INTO sqlite_sequence (name, seq) SELECT 'audit_log_rebuilt', seq FROM sqlite_sequence WHERE name = 'audit_log';

DROP TABLE audit_log;
ALTER TABLE audit_log_rebuilt RENAME TO audit_log;

CREATE INDEX audit_log_action ON audit_log (action, id);
CREATE INDEX audit_log_actor ON audit_log (actor_name COLLATE NOCASE, id);
CREATE INDEX audit_log_expires ON audit_log (expires_at);

CREATE TRIGGER audit_log_append_only BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit entries are append-only');
END;

CREATE TRIGGER audit_log_kept_until_purged BEFORE DELETE ON audit_log
WHEN OLD.expires_at > (SELECT cutoff FROM audit_purge WHERE id = 1)
BEGIN
    SELECT RAISE(ABORT, 'audit entries are kept until their retention ends');
END;
