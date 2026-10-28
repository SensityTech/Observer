-- Bumped by every change, which names the version it was made from: a change made from settings
-- another session has replaced since is refused rather than undoing theirs.
ALTER TABLE launch_settings ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
