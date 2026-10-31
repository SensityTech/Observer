-- Server tools: what the owner decided for each tool, the runs, and the journal of the files the
-- tools changed in the server folder. Every timestamp is RFC 3339 UTC with three fractional digits;
-- names are stored as observer-domain writes them. A tool's name has no CHECK, so that a new tool
-- needs no rebuild of these tables.

-- One server's settings for one tool. A missing row means the defaults: nothing at start, identical
-- copies only, escrowed resources skipped, nothing excluded.
CREATE TABLE tool_settings (
    server_id   TEXT    NOT NULL,
    tool        TEXT    NOT NULL CHECK (length(tool) BETWEEN 1 AND 64),
    at_start    TEXT    NOT NULL CHECK (at_start IN ('off', 'report', 'fix')),
    differences TEXT    NOT NULL CHECK (differences IN ('identical', 'keepLatest')),
    escrowed    TEXT    NOT NULL CHECK (escrowed IN ('skip', 'allow')),
    -- ["resource", ...], names as FXServer knows them.
    excluded    TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(excluded) AND json_type(excluded) = 'array'),
    version     INTEGER NOT NULL DEFAULT 1,
    updated_at  TEXT    NOT NULL,
    PRIMARY KEY (server_id, tool)
) STRICT, WITHOUT ROWID;

-- Every run of a tool. The last runs of each tool are kept, and any run a change below names
-- whatever its age; the full report only for the latest report and the runs a change names.
CREATE TABLE tool_runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id   TEXT    NOT NULL,
    tool        TEXT    NOT NULL CHECK (length(tool) BETWEEN 1 AND 64),
    trigger     TEXT    NOT NULL CHECK (trigger IN ('start', 'panel', 'cli')),
    -- The account that asked; NULL when Observer ran it by itself.
    actor       TEXT,
    mode        TEXT    NOT NULL CHECK (mode IN ('report', 'fix', 'heal', 'undo')),
    started_at  TEXT    NOT NULL,
    finished_at TEXT,
    outcome     TEXT    CHECK (outcome IN ('done', 'cancelled', 'timedOut', 'failed')),
    summary     TEXT    CHECK (summary IS NULL OR (json_valid(summary) AND json_type(summary) = 'object')),
    -- The SHA-256 of the plan the report holds, which a fix names to confirm what the owner saw.
    plan_sha256 TEXT    CHECK (plan_sha256 IS NULL
        OR (length(plan_sha256) = 64 AND plan_sha256 NOT GLOB '*[^0-9a-f]*')),
    report      TEXT    CHECK (report IS NULL OR (json_valid(report) AND json_type(report) = 'object')),
    CHECK ((finished_at IS NULL) = (outcome IS NULL)),
    CHECK (finished_at IS NOT NULL OR (summary IS NULL AND plan_sha256 IS NULL AND report IS NULL))
) STRICT;

CREATE INDEX tool_runs_tool ON tool_runs (server_id, tool, id);

-- The journal: one record per server folder and file a tool changed, open while the file holds the
-- change. Its original, the file before Observer touched it, is kept under data/tools/backups by
-- its SHA-256; the entries taken out add up over the runs, positions in that original. A write is
-- recorded before its temporary file exists (pending) and again once its rename landed (applied),
-- so that no change ever goes unrecorded. Runs are never deleted from under it.
CREATE TABLE tool_changes (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id         TEXT    NOT NULL,
    tool              TEXT    NOT NULL CHECK (length(tool) BETWEEN 1 AND 64),
    -- The server folder's final path when the record opened, and its identity (volume and file
    -- id), which heal and undo go by: a folder moved, or reached by another path, is the same one.
    folder_path       TEXT    NOT NULL,
    folder_id         TEXT    NOT NULL CHECK (length(folder_id) > 0),
    -- Relative to the server folder, with '/'.
    path              TEXT    NOT NULL CHECK (length(path) > 0),
    resource          TEXT    NOT NULL,
    format            TEXT    NOT NULL CHECK (format IN ('ytyp', 'modelInfo')),
    original_sha256   TEXT    NOT NULL
        CHECK (length(original_sha256) = 64 AND original_sha256 NOT GLOB '*[^0-9a-f]*'),
    -- What the file holds while the change is applied; NULL until the first write lands.
    patched_sha256    TEXT    CHECK (patched_sha256 IS NULL
        OR (length(patched_sha256) = 64 AND patched_sha256 NOT GLOB '*[^0-9a-f]*')),
    -- [{ "index", "hash", "name", "keeper": { "path", "format", "index", "sha256" }, "run" }]
    removals          TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(removals) AND json_type(removals) = 'array'),
    state             TEXT    NOT NULL CHECK (state IN ('pending', 'applied', 'closed')),
    -- While a write is under way: the temporary file's name in the changed file's folder (also in
    -- file_uploads, so that a start removes a leftover), what the file holds once the rename
    -- lands, every entry out by then, and the run writing.
    pending_temporary TEXT    CHECK (pending_temporary IS NULL OR length(pending_temporary) > 0),
    pending_sha256    TEXT    CHECK (pending_sha256 IS NULL
        OR (length(pending_sha256) = 64 AND pending_sha256 NOT GLOB '*[^0-9a-f]*')),
    pending_removals  TEXT    CHECK (pending_removals IS NULL
        OR (json_valid(pending_removals) AND json_type(pending_removals) = 'array')),
    pending_run       INTEGER REFERENCES tool_runs (id),
    close_reason      TEXT    CHECK (close_reason IN ('undone', 'healed', 'stale', 'restored', 'abandoned')),
    opened_run        INTEGER NOT NULL REFERENCES tool_runs (id),
    -- The run of the latest write that landed.
    changed_run       INTEGER REFERENCES tool_runs (id),
    closed_run        INTEGER REFERENCES tool_runs (id),
    opened_at         TEXT    NOT NULL,
    changed_at        TEXT,
    closed_at         TEXT,
    CHECK ((pending_temporary IS NULL) = (pending_sha256 IS NULL)
        AND (pending_sha256 IS NULL) = (pending_removals IS NULL)
        AND (pending_removals IS NULL) = (pending_run IS NULL)),
    CHECK ((state = 'pending') = (pending_temporary IS NOT NULL)),
    CHECK ((state = 'closed') = (close_reason IS NOT NULL)
        AND (close_reason IS NULL) = (closed_run IS NULL)
        AND (closed_run IS NULL) = (closed_at IS NULL)),
    CHECK ((changed_run IS NULL) = (changed_at IS NULL)),
    CHECK ((patched_sha256 IS NULL) = (changed_run IS NULL)),
    CHECK (state <> 'applied' OR patched_sha256 IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX tool_changes_open ON tool_changes (server_id, tool, folder_id, path)
    WHERE state IN ('pending', 'applied');
CREATE INDEX tool_changes_state ON tool_changes (state, closed_at);
-- The foreign keys' own: a run is deleted only once no change names it.
CREATE INDEX tool_changes_opened_run ON tool_changes (opened_run);
CREATE INDEX tool_changes_changed_run ON tool_changes (changed_run);
CREATE INDEX tool_changes_closed_run ON tool_changes (closed_run);
CREATE INDEX tool_changes_pending_run ON tool_changes (pending_run);
