-- A fix the owner prepared while FXServer ran: written before its next start, if a fresh scan then
-- plans exactly what the owner saw. One at most for each tool; it goes once that start has read it,
-- or once any run changed files, after which the plan is another.

CREATE TABLE tool_staged (
    server_id   TEXT    NOT NULL,
    tool        TEXT    NOT NULL CHECK (length(tool) BETWEEN 1 AND 64),
    plan_sha256 TEXT    NOT NULL CHECK (length(plan_sha256) = 64 AND plan_sha256 NOT GLOB '*[^0-9a-f]*'),
    -- The groups the plan fixes, for the page.
    fixes       INTEGER NOT NULL CHECK (fixes > 0),
    staged_at   TEXT    NOT NULL,
    staged_by   TEXT    NOT NULL,
    PRIMARY KEY (server_id, tool)
) STRICT, WITHOUT ROWID;
