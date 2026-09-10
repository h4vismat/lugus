use base64::{Engine, engine::general_purpose::STANDARD};
use lugus_app::{passages::*, *};
use lugus_financial::{
    domain::Document,
    storage::{Repository, SqliteRepository},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
struct Ids(AtomicU64);
impl IdSource for Ids {
    fn next_id(&self) -> String {
        format!("passage-{}", self.0.fetch_add(1, Ordering::Relaxed))
    }
}
fn scope() -> Scope {
    Scope {
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        run_id: None,
    }
}
fn open(db: &Path, fin: &Path, seed: u64) -> SqliteApplicationStore {
    SqliteApplicationStore::open(
        db,
        Box::new(SqliteRepository::open(fin).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(Ids(AtomicU64::new(seed))),
    )
    .unwrap()
}
fn dataset(store: &mut SqliteApplicationStore, fin: &Path, html: &str) -> DatasetHeader {
    let mut repo = SqliteRepository::open(fin).unwrap();
    let provider = ProviderIdentity {
        instance_id: "fixture".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    };
    let doc = Document {
        source_url: "https://fixture.test/filing".into(),
        media_type: "text/html".into(),
        content_base64: STANDARD.encode(html),
        retrieved_at: "2026-09-10T00:00:00Z".parse().unwrap(),
    };
    let sum = repo
        .save_document(&provider, &doc, 32 * 1024 * 1024)
        .unwrap();
    let observation = repo.document_observations(&sum).unwrap().remove(0);
    let fetch = store
        .record_fetch(&FetchResult {
            provenance: FetchProvenance {
                scope: scope(),
                provider,
                repository_id: repo.repository_identity().unwrap(),
                command: FetchCommand::Document {
                    instance_id: "fixture".into(),
                    source_url: doc.source_url,
                },
                runs: vec![],
                document: Some(observation),
                instrument_observation: None,
                binding_id: None,
            },
            error: None,
        })
        .unwrap();
    store
        .create_dataset(&scope(), &fetch.id, DatasetProjection::Document)
        .unwrap()
}
fn prepare(store: &mut SqliteApplicationStore, id: &str) -> TextRepresentation {
    let limits = TextLimits::default();
    match store
        .load_text_preparation(
            &scope(),
            id,
            &ExtractorIdentity::html_v1(),
            &limits,
            100_000,
        )
        .unwrap()
    {
        TextPreparation::Cached(h) => *h,
        TextPreparation::Input(input) => {
            let extracted = extract_html(
                &input.bytes,
                input.media_type(),
                &limits,
                &AtomicBool::new(false),
            )
            .unwrap();
            store
                .save_text_representation(
                    &scope(),
                    &PreparedText::new(*input, extracted, &limits, &AtomicBool::new(false))
                        .unwrap(),
                    100_000,
                )
                .unwrap()
        }
    }
}
fn request(id: &str) -> CreatePassageRequest {
    CreatePassageRequest {
        representation_id: id.into(),
        start: 0,
        end: 20,
        expected_text: "Revenue & cash grew.".into(),
    }
}
#[test]
fn immutable_scoped_passages_survive_changed_source_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    let h = prepare(&mut s, &d.id);
    assert_eq!(h.id, prepare(&mut s, &d.id).id);
    let p = s
        .create_passage(&scope(), &request(&h.id), &TextLimits::default(), 100_000)
        .unwrap();
    assert_eq!(
        p.id,
        s.create_passage(&scope(), &request(&h.id), &TextLimits::default(), 100_000)
            .unwrap()
            .id
    );
    let original =
        serde_json::to_value(s.resolve_passage_sources(&scope(), &p.id, 100_000).unwrap()).unwrap();
    assert_eq!(original["sources"][0]["text"], "Revenue & cash grew.");
    let newer = dataset(&mut s, &fin, "<p>Revenue fell.</p>");
    assert_ne!(h.id, prepare(&mut s, &newer.id).id);
    drop(s);
    let s = open(&db, &fin, 100);
    assert_eq!(
        s.read_passage(&scope(), &p.id, 100_000).unwrap().quote,
        "Revenue & cash grew."
    );
    assert_eq!(
        original,
        serde_json::to_value(s.resolve_passage_sources(&scope(), &p.id, 100_000).unwrap()).unwrap()
    );
    let mut wrong = scope();
    wrong.workspace_id = "other".into();
    assert_eq!(
        s.read_passage(&wrong, &p.id, 100_000).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    let other = dir.path().join("other-fin");
    let other = open(&db, &other, 200);
    assert_eq!(
        other
            .read_text_representation(&scope(), &h.id, 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
}
#[test]
fn invalid_selection_and_envelopes_leave_no_passages() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    let h = prepare(&mut s, &d.id);
    let mut r = request(&h.id);
    r.expected_text = "stale".into();
    assert_eq!(
        s.create_passage(&scope(), &r, &TextLimits::default(), 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        s.create_passage(&scope(), &request(&h.id), &TextLimits::default(), 20)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    let sql = rusqlite::Connection::open(db).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM passage_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn separate_extractors_and_datasets_never_replace_old_representations() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    let first = prepare(&mut s, &d.id);
    let mut identity = ExtractorIdentity::html_v1();
    identity.policy = "html-text-v2-fixture".into();
    let limits = TextLimits::default();
    let TextPreparation::Input(input) = s
        .load_text_preparation(&scope(), &d.id, &identity, &limits, 100_000)
        .unwrap()
    else {
        panic!("new extractor requires input")
    };
    let mut extracted = extract_html(
        &input.bytes,
        input.media_type(),
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    extracted.extractor = identity;
    let token = PreparedText::new(*input, extracted, &limits, &AtomicBool::new(false)).unwrap();
    let revised = s
        .save_text_representation(&scope(), &token, 100_000)
        .unwrap();
    assert_ne!(first.id, revised.id);
    assert_eq!(
        revised.id,
        s.save_text_representation(&scope(), &token, 100_000)
            .unwrap()
            .id
    );
    let another = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    assert_ne!(first.id, prepare(&mut s, &another.id).id);
    let mut wrong = scope();
    wrong.workspace_id = "wrong".into();
    assert_eq!(
        s.load_text_preparation(
            &wrong,
            &d.id,
            &ExtractorIdentity::html_v1(),
            &limits,
            100_000
        )
        .unwrap_err()
        .kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        s.save_text_representation(&wrong, &token, 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
}
#[test]
fn overlapping_normalized_contributors_and_unicode_ranges_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>é <b> </b> cash</p><p>next</p>");
    let h = prepare(&mut s, &d.id);
    assert!(
        s.read_text_page(&scope(), &h.id, 1, 2, &TextLimits::default(), 100_000)
            .is_err()
    );
    assert!(
        s.read_text_page(&scope(), &h.id, 1, 1, &TextLimits::default(), 100_000)
            .is_err()
    );
    let r = CreatePassageRequest {
        representation_id: h.id.clone(),
        start: 0,
        end: 7,
        expected_text: "é cash".into(),
    };
    let p = s
        .create_passage(&scope(), &r, &TextLimits::default(), 100_000)
        .unwrap();
    let sources = s.resolve_passage_sources(&scope(), &p.id, 100_000).unwrap();
    assert_eq!(
        sources
            .sources
            .iter()
            .map(|s| s.text.as_str())
            .collect::<String>(),
        "é   cash"
    );
    assert_eq!(
        p.mappings
            .iter()
            .filter(|m| m.kind == MappingKind::Normalized)
            .count(),
        3
    );
    let mut conflicting = r;
    conflicting.expected_text = "other".into();
    assert_eq!(
        s.create_passage(&scope(), &conflicting, &TextLimits::default(), 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    let mut another = scope();
    another.request_id = "separator".into();
    let synthetic = CreatePassageRequest {
        representation_id: h.id,
        start: 7,
        end: 8,
        expected_text: "\n".into(),
    };
    assert_eq!(
        s.create_passage(&another, &synthetic, &TextLimits::default(), 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
}
#[test]
fn small_reads_touch_only_overlapping_chunks_and_reject_accessed_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(
        &mut s,
        &fin,
        &format!("<p>Revenue &amp; cash grew.{}tail</p>", "x".repeat(100_000)),
    );
    let h = prepare(&mut s, &d.id);
    let p = s
        .create_passage(&scope(), &request(&h.id), &TextLimits::default(), 100_000)
        .unwrap();
    let sql = rusqlite::Connection::open(&db).unwrap();
    assert!(
        sql.query_row("SELECT count(*) FROM text_chunks WHERE node>=0", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
            > 20
    );
    sql.execute(
        "UPDATE text_chunks SET text=zeroblob(2000000) WHERE start>90000",
        [],
    )
    .unwrap();
    assert_eq!(
        s.read_text_page(&scope(), &h.id, 0, 7, &TextLimits::default(), 1000)
            .unwrap()
            .text,
        "Revenue"
    );
    assert_eq!(
        s.resolve_passage_sources(&scope(), &p.id, 100_000)
            .unwrap()
            .sources[0]
            .text,
        "Revenue & cash grew."
    );
    sql.execute(
        "UPDATE text_chunks SET text=zeroblob(2000000) WHERE node=-1 AND start=0",
        [],
    )
    .unwrap();
    assert_eq!(
        s.read_text_page(&scope(), &h.id, 0, 7, &TextLimits::default(), 1000)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
}
#[test]
fn preparation_rechecks_dataset_association_and_rejects_forged_output() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    let limits = TextLimits::default();
    let TextPreparation::Input(input) = s
        .load_text_preparation(
            &scope(),
            &d.id,
            &ExtractorIdentity::html_v1(),
            &limits,
            100_000,
        )
        .unwrap()
    else {
        panic!("input")
    };
    let mut extracted = extract_html(
        &input.bytes,
        input.media_type(),
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    extracted.text = "forged".into();
    assert!(PreparedText::new(*input, extracted, &limits, &AtomicBool::new(false)).is_err());
    let TextPreparation::Input(input) = s
        .load_text_preparation(
            &scope(),
            &d.id,
            &ExtractorIdentity::html_v1(),
            &limits,
            100_000,
        )
        .unwrap()
    else {
        panic!("input")
    };
    let extracted = extract_html(
        &input.bytes,
        input.media_type(),
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    let token = PreparedText::new(*input, extracted, &limits, &AtomicBool::new(false)).unwrap();
    let sql = rusqlite::Connection::open(db).unwrap();
    sql.execute("UPDATE app_records SET payload=json_set(payload,'$.limitations',json('[\"changed\"]')) WHERE id=?1",[d.id]).unwrap();
    assert_eq!(
        s.save_text_representation(&scope(), &token, 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM text_representations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn racing_connections_deduplicate_both_representations_and_passages() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    drop(s);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs = (0..2)
        .map(|i| {
            let db = db.clone();
            let fin = fin.clone();
            let dataset = d.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut s = open(&db, &fin, 100 + i * 100);
                let limits = TextLimits::default();
                let TextPreparation::Input(input) = s
                    .load_text_preparation(
                        &scope(),
                        &dataset,
                        &ExtractorIdentity::html_v1(),
                        &limits,
                        100_000,
                    )
                    .unwrap()
                else {
                    panic!("input")
                };
                let extracted = extract_html(
                    &input.bytes,
                    input.media_type(),
                    &limits,
                    &AtomicBool::new(false),
                )
                .unwrap();
                let token =
                    PreparedText::new(*input, extracted, &limits, &AtomicBool::new(false)).unwrap();
                barrier.wait();
                let h = s
                    .save_text_representation(&scope(), &token, 100_000)
                    .unwrap();
                barrier.wait();
                let p = s
                    .create_passage(&scope(), &request(&h.id), &limits, 100_000)
                    .unwrap();
                (h.id, p.id)
            })
        })
        .collect::<Vec<_>>();
    let results = jobs
        .into_iter()
        .map(|job| job.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results[0], results[1]);
    let sql = rusqlite::Connection::open(db).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM text_representations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM passage_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn corrupted_source_lengths_checksums_and_mapping_rows_fail_safely() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    let h = prepare(&mut s, &d.id);
    let p = s
        .create_passage(&scope(), &request(&h.id), &TextLimits::default(), 100_000)
        .unwrap();
    let sql = rusqlite::Connection::open(&db).unwrap();
    sql.execute("UPDATE text_nodes SET bytes=bytes+1", [])
        .unwrap();
    assert!(s.resolve_passage_sources(&scope(), &p.id, 100_000).is_err());
    sql.execute("UPDATE text_nodes SET bytes=bytes-1", [])
        .unwrap();
    sql.execute(
        "UPDATE text_chunks SET text='Revenue & cash lost.' WHERE node>=0",
        [],
    )
    .unwrap();
    assert_eq!(
        s.resolve_passage_sources(&scope(), &p.id, 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::Storage
    );
    sql.execute("UPDATE text_mappings SET payload=zeroblob(2000000)", [])
        .unwrap();
    assert_eq!(
        s.read_passage(&scope(), &p.id, 100_000).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}
#[test]
fn full_source_response_checks_escaping_and_exact_envelope_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    let h = prepare(&mut s, &d.id);
    let p = s
        .create_passage(&scope(), &request(&h.id), &TextLimits::default(), 100_000)
        .unwrap();
    let result = s.resolve_passage_sources(&scope(), &p.id, 100_000).unwrap();
    let size = serde_json::to_vec(&result).unwrap().len();
    assert!(s.resolve_passage_sources(&scope(), &p.id, size).is_ok());
    assert_eq!(
        s.resolve_passage_sources(&scope(), &p.id, size - 1)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        s.read_text_page(&scope(), &h.id, 0, 7, &TextLimits::default(), 7)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
}
#[test]
fn failed_atomic_save_leaves_original_document_and_no_partial_text() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let html = "<p>Revenue &amp; cash grew.</p>";
    let d = dataset(&mut s, &fin, html);
    let limits = TextLimits::default();
    let TextPreparation::Input(input) = s
        .load_text_preparation(
            &scope(),
            &d.id,
            &ExtractorIdentity::html_v1(),
            &limits,
            100_000,
        )
        .unwrap()
    else {
        panic!("input")
    };
    let extracted = extract_html(
        &input.bytes,
        input.media_type(),
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    let token = PreparedText::new(*input, extracted, &limits, &AtomicBool::new(false)).unwrap();
    let sql = rusqlite::Connection::open(db).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_mapping BEFORE INSERT ON text_mappings BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert_eq!(
        s.save_text_representation(&scope(), &token, 100_000)
            .unwrap_err()
            .kind,
        ErrorKind::Storage
    );
    for table in [
        "text_representations",
        "text_chunks",
        "text_nodes",
        "text_mappings",
    ] {
        assert_eq!(
            sql.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        s.read_document(&scope(), &d.id, 0, 100).unwrap().bytes,
        html.as_bytes()
    );
    sql.execute_batch("DROP TRIGGER fail_mapping").unwrap();
    assert!(
        s.save_text_representation(&scope(), &token, 100_000)
            .is_ok()
    );
}

#[test]
fn retries_return_original_without_allocating_new_ids() {
    struct NoIds;
    impl IdSource for NoIds {
        fn next_id(&self) -> String {
            String::new()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = open(&db, &fin, 1);
    let d = dataset(&mut s, &fin, "<p>Revenue &amp; cash grew.</p>");
    let limits = TextLimits::default();
    let TextPreparation::Input(input) = s
        .load_text_preparation(
            &scope(),
            &d.id,
            &ExtractorIdentity::html_v1(),
            &limits,
            100_000,
        )
        .unwrap()
    else {
        panic!("input")
    };
    let extracted = extract_html(
        &input.bytes,
        input.media_type(),
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    let token = PreparedText::new(*input, extracted, &limits, &AtomicBool::new(false)).unwrap();
    let h = s
        .save_text_representation(&scope(), &token, 100_000)
        .unwrap();
    let p = s
        .create_passage(&scope(), &request(&h.id), &limits, 100_000)
        .unwrap();
    drop(s);
    let mut s = SqliteApplicationStore::open(
        db,
        Box::new(SqliteRepository::open(fin).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(NoIds),
    )
    .unwrap();
    assert_eq!(
        s.save_text_representation(&scope(), &token, 100_000)
            .unwrap()
            .id,
        h.id
    );
    assert_eq!(
        s.create_passage(&scope(), &request(&h.id), &limits, 100_000)
            .unwrap()
            .id,
        p.id
    );
}
