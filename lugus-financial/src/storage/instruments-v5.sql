-- Each successful retrieval is an independent immutable observation, including
-- identical repeated payloads. Provider identity is retained on the evidence row.
CREATE TABLE instrument_observations (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 provider_id TEXT NOT NULL,
 provider TEXT NOT NULL,
 request TEXT NOT NULL,
 payload TEXT NOT NULL,
 recorded_at TEXT NOT NULL
);
CREATE TRIGGER instrument_observations_no_update BEFORE UPDATE ON instrument_observations BEGIN
 SELECT RAISE(ABORT, 'instrument observations are immutable');
END;
CREATE TRIGGER instrument_observations_no_delete BEFORE DELETE ON instrument_observations BEGIN
 SELECT RAISE(ABORT, 'instrument observations are immutable');
END;
PRAGMA user_version = 5;
