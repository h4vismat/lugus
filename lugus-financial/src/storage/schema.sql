CREATE TABLE providers (id TEXT PRIMARY KEY, identity TEXT NOT NULL);
CREATE TABLE companies (namespace TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(namespace,value));
CREATE TABLE runs (id INTEGER PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id), query TEXT NOT NULL, operation TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('running','complete','failed')), filings_cursor TEXT, facts_cursor TEXT, error TEXT, started_at TEXT NOT NULL, finished_at TEXT);
CREATE TABLE observations (id INTEGER PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id), kind TEXT NOT NULL, fingerprint TEXT NOT NULL, payload TEXT NOT NULL, metric TEXT, UNIQUE(provider_id,kind,fingerprint));
CREATE TABLE run_observations (run_id INTEGER NOT NULL REFERENCES runs(id), observation_id INTEGER NOT NULL REFERENCES observations(id), retrieved_at TEXT NOT NULL, PRIMARY KEY(run_id,observation_id));
CREATE TABLE document_content (checksum TEXT PRIMARY KEY, content BLOB NOT NULL);
CREATE TABLE document_observations (id INTEGER PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id), checksum TEXT NOT NULL REFERENCES document_content(checksum), source_url TEXT NOT NULL, media_type TEXT NOT NULL, retrieved_at TEXT NOT NULL);
PRAGMA user_version = 1;
