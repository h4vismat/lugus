CREATE TABLE historical_runs (
 id INTEGER PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id),
 query TEXT NOT NULL, status TEXT NOT NULL, manifest TEXT, cursor TEXT,
 seen_cursors TEXT NOT NULL DEFAULT '[]', last_date TEXT, row_count INTEGER NOT NULL DEFAULT 0,
 started_at TEXT NOT NULL, finished_at TEXT, error TEXT
);
CREATE TABLE historical_observations (
 id INTEGER PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id),
 fingerprint TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(provider_id,fingerprint)
);
CREATE TABLE historical_run_days (
 run_id INTEGER NOT NULL REFERENCES historical_runs(id), ordinal INTEGER NOT NULL,
 date TEXT NOT NULL, observation_id INTEGER NOT NULL REFERENCES historical_observations(id),
 PRIMARY KEY(run_id,ordinal), UNIQUE(run_id,date)
);
CREATE INDEX historical_days_by_date ON historical_run_days(run_id,date);
PRAGMA user_version=6;
