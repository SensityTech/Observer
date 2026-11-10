-- The launch's cfg as it was before each save from the panel: the latest few of each file, with
-- who replaced them and when. The content is the file's own bytes, its secrets included, as the
-- server folder held them. Times are RFC 3339 UTC with three fractional digits.

CREATE TABLE cfg_version (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id   TEXT    NOT NULL,
    -- Relative to the server folder, with '/'.
    file        TEXT    NOT NULL CHECK (length(file) BETWEEN 1 AND 1024),
    replaced_at TEXT    NOT NULL,
    replaced_by TEXT    NOT NULL,
    lines       INTEGER NOT NULL CHECK (lines >= 0),
    content     BLOB    NOT NULL CHECK (length(content) <= 1048576)
) STRICT;

CREATE INDEX cfg_version_by_file ON cfg_version (server_id, file, id);
