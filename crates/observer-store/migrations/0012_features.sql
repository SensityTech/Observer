-- The optional features the owner turned on or off from the panel. A feature without a row keeps
-- its default, on; observer.toml can still hold it off whatever its row says. Keys are stored as
-- observer-domain writes them, without a CHECK, so that a new feature needs no rebuild.
CREATE TABLE features (
    key        TEXT    PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 64),
    enabled    INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    changed_at TEXT    NOT NULL
) STRICT, WITHOUT ROWID;
