-- Whether Observer restarts a server whose main loop or HTTP endpoints stay silent, and after how
-- long, in seconds: whole minutes, as the panel sets it. Servers saved before get it on, after six
-- minutes: a big server's first start can keep its main loop busy that long while it builds its
-- caches. A crash restarts the server whatever these say.
ALTER TABLE launch_settings ADD COLUMN hang_detection INTEGER NOT NULL DEFAULT 1
    CHECK (hang_detection IN (0, 1));
ALTER TABLE launch_settings ADD COLUMN hang_timeout_seconds INTEGER NOT NULL DEFAULT 360
    CHECK (hang_timeout_seconds BETWEEN 60 AND 3600 AND hang_timeout_seconds % 60 = 0);
