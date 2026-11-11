-- How long a stop waits, at most, for the resources that hold it through the bridge, in seconds,
-- before FXServer quits anyway. Servers saved before get a minute. Five seconds is the notice every
-- stop gives: at that, a stop waits for no resource longer than it did before.
ALTER TABLE launch_settings ADD COLUMN stop_hold_seconds INTEGER NOT NULL DEFAULT 60
    CHECK (stop_hold_seconds BETWEEN 5 AND 600);
