-- One row identifying this installation. Session cookies will carry this id, so
-- two Observer instances on one host never read each other's cookies.
CREATE TABLE instance (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    instance_id TEXT    NOT NULL,
    created_at  TEXT    NOT NULL
) STRICT;

INSERT INTO instance (id, instance_id, created_at)
VALUES (1, lower(hex(randomblob(8))), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
