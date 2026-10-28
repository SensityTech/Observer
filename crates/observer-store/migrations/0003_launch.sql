-- How Observer launches FXServer, one row per server: 'default' until Observer manages several.
-- Only the owner changes it, through checks that leave nothing here FXServer could not run with.
CREATE TABLE launch_settings (
    server_id       TEXT    PRIMARY KEY,
    build_path      TEXT    NOT NULL,
    data_path       TEXT    NOT NULL,
    -- Relative to data_path.
    cfg_file        TEXT    NOT NULL,
    onesync         TEXT    NOT NULL CHECK (onesync IN ('off', 'on', 'legacy')),
    auto_start      INTEGER NOT NULL CHECK (auto_start IN (0, 1)),
    txadmin_events  INTEGER NOT NULL CHECK (txadmin_events IN (0, 1)),
    -- [{"name": "...", "value": "..."}], in the order they go on the command line.
    extra_arguments TEXT    NOT NULL DEFAULT '[]'
        CHECK (json_valid(extra_arguments) AND json_type(extra_arguments) = 'array'),
    updated_at      TEXT    NOT NULL
) STRICT, WITHOUT ROWID;
