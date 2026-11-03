-- The labels the server's own resources gave a player through the bridge: a character's name, an id
-- of their own. Display data, kept apart from the identity: never matched, never linked, never a
-- player's name. An absent name or id is ''.

CREATE TABLE player_label (
    player       INTEGER NOT NULL REFERENCES player (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL CHECK (length(name) <= 64),
    label_id     TEXT    NOT NULL CHECK (length(label_id) <= 32),
    -- The resource that gave it last.
    resource     TEXT    CHECK (length(resource) <= 64),
    first_seen   TEXT    NOT NULL,
    last_seen    TEXT    NOT NULL,
    PRIMARY KEY (player, name, label_id),
    CHECK (name <> '' OR label_id <> '')
) STRICT, WITHOUT ROWID;

CREATE INDEX player_label_by_name ON player_label (name COLLATE NOCASE, player);
CREATE INDEX player_label_by_id ON player_label (label_id COLLATE NOCASE, player);
CREATE INDEX player_label_last_seen ON player_label (last_seen);
