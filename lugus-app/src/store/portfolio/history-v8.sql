CREATE TABLE portfolio_history_jobs (
 id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE,
 portfolio_id TEXT NOT NULL REFERENCES portfolios(id),
 input TEXT NOT NULL, key TEXT NOT NULL, payload TEXT NOT NULL,
 status TEXT NOT NULL, ordinal_count INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX one_active_portfolio_history ON portfolio_history_jobs(portfolio_id) WHERE status='running';
CREATE TABLE portfolio_performance_days (
 result_id TEXT NOT NULL REFERENCES portfolio_history_jobs(id),ordinal INTEGER NOT NULL,date TEXT NOT NULL,payload TEXT NOT NULL,
 PRIMARY KEY(result_id,ordinal),UNIQUE(result_id,date)
);
CREATE TABLE portfolio_history_evidence (
 result_id TEXT NOT NULL REFERENCES portfolio_history_jobs(id),ordinal INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(result_id,ordinal)
);
PRAGMA user_version=8;
