-- Which FXServer build runs, one row per server: the selected build, the one waiting for the next
-- scheduled restart, the last build that passed its probation, a build on probation with the last
-- healthy one when it opened, the highest build ever run (going back below 6449 after it may lose
-- the server's KVP data), the latest thing Observer did by itself, and the build folder of the
-- launch settings from before builds lived in Observer's data folder. Instants are UTC text.
CREATE TABLE artifact_state (
    server_id         TEXT    PRIMARY KEY,
    selected_build    INTEGER CHECK (selected_build BETWEEN 1000 AND 9999999),
    pending_build     INTEGER CHECK (pending_build BETWEEN 1000 AND 9999999),
    pending_by        TEXT,
    pending_at        TEXT,
    last_healthy      INTEGER CHECK (last_healthy BETWEEN 1000 AND 9999999),
    last_healthy_at   TEXT,
    probation_build   INTEGER CHECK (probation_build BETWEEN 1000 AND 9999999),
    probation_from    INTEGER CHECK (probation_from BETWEEN 1000 AND 9999999),
    highest_run       INTEGER CHECK (highest_run BETWEEN 1000 AND 9999999),
    notice_kind       TEXT    CHECK (notice_kind IN ('rolledBack', 'probationFailed', 'pendingDropped')),
    notice_build      INTEGER CHECK (notice_build BETWEEN 1000 AND 9999999),
    notice_other      INTEGER CHECK (notice_other BETWEEN 1000 AND 9999999),
    notice_reason     TEXT,
    notice_at         TEXT,
    legacy_build_path TEXT,
    version           INTEGER NOT NULL DEFAULT 1,
    updated_at        TEXT    NOT NULL,
    CHECK ((pending_build IS NULL) = (pending_by IS NULL) AND (pending_by IS NULL) = (pending_at IS NULL)),
    CHECK ((last_healthy IS NULL) = (last_healthy_at IS NULL)),
    CHECK (probation_build IS NOT NULL OR probation_from IS NULL),
    CHECK ((notice_kind IS NULL) = (notice_build IS NULL) AND (notice_build IS NULL) = (notice_reason IS NULL)
        AND (notice_reason IS NULL) = (notice_at IS NULL)),
    CHECK (notice_kind IS NOT NULL OR notice_other IS NULL)
) STRICT, WITHOUT ROWID;

-- The folder the launch settings named moves here: FXServer now runs a build chosen among those
-- installed, and the launch settings only say how it runs. Their version stays as it is.
INSERT INTO artifact_state (server_id, legacy_build_path, updated_at)
    SELECT server_id, build_path, updated_at FROM launch_settings WHERE trim(build_path) <> '';
ALTER TABLE launch_settings DROP COLUMN build_path;

-- How builds are kept and tried, one row per server. A missing row means the defaults: three
-- builds kept, the automatic return on, no test branch.
CREATE TABLE artifact_settings (
    server_id     TEXT    PRIMARY KEY,
    keep_builds   INTEGER NOT NULL CHECK (keep_builds BETWEEN 1 AND 20),
    auto_rollback INTEGER NOT NULL CHECK (auto_rollback IN (0, 1)),
    -- ["feature/...", ...], master aside.
    branches      TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(branches) AND json_type(branches) = 'array' AND json_array_length(branches) <= 10),
    version       INTEGER NOT NULL DEFAULT 1,
    updated_at    TEXT    NOT NULL
) STRICT, WITHOUT ROWID;

-- The downloadable builds the catalog has listed, so that a build no source lists any more can
-- still be offered, with what was learnt of it then.
CREATE TABLE artifact_builds (
    platform       TEXT    NOT NULL CHECK (platform IN ('windows', 'linux')),
    build          INTEGER NOT NULL CHECK (build BETWEEN 1000 AND 9999999),
    commit_sha     TEXT    NOT NULL CHECK (length(commit_sha) = 40),
    branch         TEXT    NOT NULL CHECK (length(branch) BETWEEN 1 AND 200),
    url            TEXT    NOT NULL,
    published_at   TEXT,
    size           INTEGER CHECK (size >= 0),
    first_seen_at  TEXT    NOT NULL,
    last_listed_at TEXT    NOT NULL,
    PRIMARY KEY (platform, build)
) STRICT, WITHOUT ROWID;

-- The SHA-256 of each build's archive the first time it was installed. Cfx.re publishes none, so a
-- later download that differs is refused. Kept when the build is deleted: that is the point.
CREATE TABLE artifact_fingerprints (
    platform       TEXT    NOT NULL CHECK (platform IN ('windows', 'linux')),
    build          INTEGER NOT NULL CHECK (build BETWEEN 1000 AND 9999999),
    archive_sha256 TEXT    NOT NULL
        CHECK (length(archive_sha256) = 64 AND archive_sha256 NOT GLOB '*[^0-9a-f]*'),
    first_at       TEXT    NOT NULL,
    PRIMARY KEY (platform, build)
) STRICT, WITHOUT ROWID;
