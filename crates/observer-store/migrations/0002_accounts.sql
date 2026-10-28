-- Accounts, sessions, login throttling and the audit log. Every timestamp is RFC 3339 UTC with
-- exactly three fractional digits, so timestamps compare correctly as text.

-- AUTOINCREMENT on users and sessions: an id names one account or one session forever, so an
-- audit entry or a stale edit never points at whoever came after a deletion.
CREATE TABLE users (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    username             TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    password_hash        TEXT    NOT NULL,
    is_owner             INTEGER NOT NULL DEFAULT 0 CHECK (is_owner IN (0, 1)),
    -- A big-endian bitset without leading zero bytes; empty means no permission. The owner holds
    -- every permission whatever is stored here.
    permissions          BLOB    NOT NULL DEFAULT x'',
    must_change_password INTEGER NOT NULL DEFAULT 0 CHECK (must_change_password IN (0, 1)),
    -- When a temporary password stops working; NULL once the user chose their own.
    password_expires_at  TEXT,
    -- A session only lives while it carries its user's current epoch.
    auth_epoch           INTEGER NOT NULL DEFAULT 0,
    version              INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT    NOT NULL,
    last_login_at        TEXT
) STRICT;

CREATE UNIQUE INDEX users_single_owner ON users (is_owner) WHERE is_owner = 1;

-- The cookie token and the session key are only stored as SHA-256 hashes.
CREATE TABLE sessions (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id            INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash         BLOB    NOT NULL UNIQUE,
    key_hash           BLOB    NOT NULL,
    auth_epoch         INTEGER NOT NULL,
    created_at         TEXT    NOT NULL,
    last_seen_at       TEXT    NOT NULL,
    expires_at         TEXT    NOT NULL,
    reauthenticated_at TEXT,
    ip                 TEXT,
    user_agent         TEXT
) STRICT;

CREATE INDEX sessions_user ON sessions (user_id);

CREATE TABLE setup_tokens (
    token_hash BLOB PRIMARY KEY,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
) STRICT, WITHOUT ROWID;

-- Failed logins per username and client network. The lock lives here rather than in memory so
-- that `observer user unlock` can lift it while Observer runs.
CREATE TABLE login_failures (
    username          TEXT    NOT NULL COLLATE NOCASE,
    network           TEXT    NOT NULL,
    failures          INTEGER NOT NULL,
    window_started_at TEXT    NOT NULL,
    locked_until      TEXT,
    lockouts          INTEGER NOT NULL DEFAULT 0,
    -- Failures not in the audit log yet. They are written there as one entry per window.
    unaudited         INTEGER NOT NULL DEFAULT 0,
    unaudited_since   TEXT,
    last_failure_at   TEXT    NOT NULL,
    PRIMARY KEY (username, network)
) STRICT, WITHOUT ROWID;

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT, WITHOUT ROWID;

-- AUTOINCREMENT keeps purged ids from coming back, which keyset pagination relies on.
CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         TEXT    NOT NULL,
    actor_kind TEXT    NOT NULL CHECK (actor_kind IN ('user', 'local', 'cli', 'system', 'anonymous')),
    actor_id   INTEGER,
    actor_name TEXT,
    ip         TEXT,
    user_agent TEXT    CHECK (length(CAST(user_agent AS BLOB)) <= 256),
    action     TEXT    NOT NULL,
    target     TEXT,
    outcome    TEXT    NOT NULL CHECK (outcome IN ('success', 'failure', 'denied')),
    details    TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(details)),
    -- Set from the retention in force when the entry is written, so lowering the retention later
    -- never shortens how long an existing entry is kept.
    expires_at TEXT    NOT NULL
) STRICT;

CREATE INDEX audit_log_action ON audit_log (action, id);
CREATE INDEX audit_log_actor ON audit_log (actor_name COLLATE NOCASE, id);
CREATE INDEX audit_log_expires ON audit_log (expires_at);

-- How far the retention job has gone: only entries that expired before the cutoff can be
-- deleted. The cutoff never moves back and never passes the present.
CREATE TABLE audit_purge (
    id     INTEGER PRIMARY KEY CHECK (id = 1),
    cutoff TEXT    NOT NULL
) STRICT;

INSERT INTO audit_purge (id, cutoff) VALUES (1, '1970-01-01T00:00:00.000Z');

CREATE TRIGGER audit_log_append_only BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit entries are append-only');
END;

CREATE TRIGGER audit_log_kept_until_purged BEFORE DELETE ON audit_log
WHEN OLD.expires_at > (SELECT cutoff FROM audit_purge WHERE id = 1)
BEGIN
    SELECT RAISE(ABORT, 'audit entries are kept until their retention ends');
END;

CREATE TRIGGER audit_purge_moves_forward BEFORE UPDATE ON audit_purge
WHEN NEW.cutoff < OLD.cutoff OR NEW.cutoff > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
BEGIN
    SELECT RAISE(ABORT, 'the audit cutoff only moves forward, up to the present');
END;

CREATE TRIGGER audit_purge_kept BEFORE DELETE ON audit_purge
BEGIN
    SELECT RAISE(ABORT, 'the audit cutoff cannot be removed');
END;
