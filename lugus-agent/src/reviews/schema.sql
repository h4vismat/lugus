CREATE TABLE thesis_revisions (
    thesis_id TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0), payload TEXT NOT NULL,
    PRIMARY KEY(thesis_id, revision)
);
CREATE TABLE reviews (
    id TEXT PRIMARY KEY, thesis_id TEXT NOT NULL, thesis_revision INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued','running','completed','interrupted','failed','blocked')),
    payload TEXT NOT NULL,
    FOREIGN KEY(thesis_id, thesis_revision) REFERENCES thesis_revisions(thesis_id, revision)
);
CREATE UNIQUE INDEX one_running_review ON reviews(thesis_id) WHERE status = 'running';
CREATE TABLE assessments (
    sequence INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, review_id TEXT NOT NULL UNIQUE REFERENCES reviews(id),
    thesis_id TEXT NOT NULL, payload TEXT NOT NULL
);
PRAGMA application_id = 1280651858;
PRAGMA user_version = 1;
