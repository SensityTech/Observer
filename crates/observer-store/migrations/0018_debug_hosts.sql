-- The hosts each resource called over HTTP, as the Debug feature saw them: the host alone, never
-- a path, a query or a body. `known` once the owner said they know it, or when it was first seen
-- during the day Observer spends learning them, so that a host called for the first time later
-- stands out.
CREATE TABLE debug_hosts (
    resource   TEXT    NOT NULL CHECK (length(resource) BETWEEN 1 AND 128),
    host       TEXT    NOT NULL CHECK (length(host) BETWEEN 1 AND 128),
    first_seen TEXT    NOT NULL,
    known      INTEGER NOT NULL CHECK (known IN (0, 1)),
    PRIMARY KEY (resource, host)
) STRICT, WITHOUT ROWID;
