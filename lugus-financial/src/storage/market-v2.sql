CREATE TABLE market_runs (
 id INTEGER PRIMARY KEY,
 provider_id TEXT NOT NULL REFERENCES providers(id),
 query TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('running','complete','failed')),
 cursor TEXT,
 coverage TEXT,
 last_date TEXT,
 seen_cursors TEXT NOT NULL DEFAULT '[]',
 error TEXT,
 started_at TEXT NOT NULL,
 finished_at TEXT
);
CREATE TABLE price_observations (
 id INTEGER PRIMARY KEY,
 provider_id TEXT NOT NULL REFERENCES providers(id),
 fingerprint TEXT NOT NULL,
 payload TEXT NOT NULL,
 UNIQUE(provider_id,fingerprint)
);
CREATE TABLE market_run_observations (
 run_id INTEGER NOT NULL REFERENCES market_runs(id),
 observation_id INTEGER NOT NULL REFERENCES price_observations(id),
 retrieved_at TEXT NOT NULL,
 PRIMARY KEY(run_id,observation_id)
);
PRAGMA user_version = 2;
