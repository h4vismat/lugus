CREATE TABLE catalog_companies (id INTEGER PRIMARY KEY AUTOINCREMENT, identity_scope TEXT NOT NULL, identifier TEXT NOT NULL, UNIQUE(identity_scope,identifier));
CREATE TABLE resolution_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, provider TEXT NOT NULL, request TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('running','complete','failed')), snapshot TEXT, coverage TEXT, cursor TEXT, seen_cursors TEXT NOT NULL DEFAULT '[]', error TEXT, failure TEXT, started_at TEXT NOT NULL, finished_at TEXT);
CREATE TABLE resolution_pages (id INTEGER PRIMARY KEY AUTOINCREMENT, run_id INTEGER NOT NULL REFERENCES resolution_runs(id), request_cursor TEXT, next_cursor TEXT, recorded_at TEXT NOT NULL);
CREATE TABLE catalog_observations (id INTEGER PRIMARY KEY AUTOINCREMENT, company_id INTEGER NOT NULL REFERENCES catalog_companies(id), provider TEXT NOT NULL, fingerprint TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(company_id,provider,fingerprint));
CREATE TABLE catalog_retrievals (id INTEGER PRIMARY KEY AUTOINCREMENT, page_id INTEGER NOT NULL REFERENCES resolution_pages(id), observation_id INTEGER NOT NULL REFERENCES catalog_observations(id), retrieved_at TEXT NOT NULL, recorded_at TEXT NOT NULL, match_reasons TEXT NOT NULL);
CREATE INDEX catalog_retrieval_observation ON catalog_retrievals(observation_id);
CREATE TABLE catalog_selections (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 retrieval_id INTEGER NOT NULL REFERENCES catalog_retrievals(id),
 entry TEXT NOT NULL,
 run_status TEXT NOT NULL,
 snapshot TEXT,
 coverage TEXT,
 error TEXT,
 failure TEXT,
 selected_at TEXT NOT NULL
);
PRAGMA user_version = 3;
