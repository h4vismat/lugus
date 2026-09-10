-- Existing cross-capability ties are ordered deterministically, not as source authority.
CREATE TABLE repository_identity (singleton INTEGER PRIMARY KEY CHECK(singleton=1), identity TEXT NOT NULL);
INSERT INTO repository_identity VALUES(1,lower(hex(randomblob(16))));
CREATE TABLE ingestion_chronology (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind IN ('financial','market')),
 run_id INTEGER NOT NULL,
 UNIQUE(kind,run_id)
);
INSERT INTO ingestion_chronology(kind,run_id)
 SELECT kind,id FROM (
 SELECT 'financial' AS kind,id,started_at FROM runs
 UNION ALL SELECT 'market',id,started_at FROM market_runs
 ) ORDER BY started_at,kind,id;
CREATE TRIGGER financial_ingestion_sequence AFTER INSERT ON runs BEGIN
 INSERT INTO ingestion_chronology(kind,run_id) VALUES('financial',NEW.id);
END;
CREATE TRIGGER market_ingestion_sequence AFTER INSERT ON market_runs BEGIN
 INSERT INTO ingestion_chronology(kind,run_id) VALUES('market',NEW.id);
END;
PRAGMA user_version = 4;
