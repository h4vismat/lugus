#[path = "support/passage_fixture.rs"]
mod fixture;
use fixture::*;
use lugus_app::{passages::*, *};
#[tokio::test]
async fn manual_preparation_and_strict_agent_tools_share_scoped_immutable_text() {
    use lugus_agent::tools::{ToolCall, ToolExecutor};
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("fin");
    let mut store = open(&dir.path().join("app"), &fin, 1);
    let d = dataset(&mut store, &fin, "<p>Revenue &amp; cash grew.</p>");
    let app = application(store, &fin).await;
    let header = app.prepare_text(&scope(), &d.id).await.unwrap();
    assert_eq!(
        app.prepare_text(&scope(), &d.id).await.unwrap().id,
        header.id
    );
    assert_eq!(
        app.read_text(&scope(), &header.id, 0, 20)
            .await
            .unwrap()
            .text,
        "Revenue & cash grew."
    );
    assert_eq!(
        app.text_header(&scope(), &header.id).await.unwrap().id,
        header.id
    );
    let passage = app
        .create_passage(
            &scope(),
            CreatePassageRequest {
                representation_id: header.id.clone(),
                start: 0,
                end: 20,
                expected_text: "Revenue & cash grew.".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        app.read_passage(&scope(), &passage.id).await.unwrap().quote,
        passage.quote
    );
    assert_eq!(
        app.resolve_passage(&scope(), &passage.id)
            .await
            .unwrap()
            .sources[0]
            .text,
        "Revenue & cash grew."
    );
    let mut agent_scope = scope();
    agent_scope.run_id = Some("run".into());
    let executor = ResearchExecutor::new(app.clone(), agent_scope).unwrap();
    let call = |name: &str, arguments| ToolCall {
        run_id: "run".into(),
        call_id: "tool".into(),
        name: name.into(),
        arguments,
    };
    assert!(
        executor
            .execute(call(
                "lugus_read_passage",
                serde_json::json!({"passage_id": passage.id})
            ))
            .await
            .success
    );
    for args in [
        serde_json::json!({"dataset_id":d.id,"workspace_id":"forged"}),
        serde_json::json!({"dataset_id":{"id":d.id}}),
    ] {
        assert!(
            !executor
                .execute(call("lugus_prepare_text", args))
                .await
                .success
        );
    }
    for (name, arguments, expected) in [
        (
            "lugus_prepare_text",
            serde_json::json!({"dataset_id":d.id}),
            "text_checksum",
        ),
        (
            "lugus_text_header",
            serde_json::json!({"representation_id":header.id}),
            "text_checksum",
        ),
        (
            "lugus_read_text",
            serde_json::json!({"representation_id":header.id,"start":0,"end":20}),
            "Revenue & cash grew.",
        ),
        (
            "lugus_create_passage",
            serde_json::json!({"request":{"representation_id":header.id,"start":0,"end":20,"expected_text":"Revenue & cash grew."}}),
            "quote_checksum",
        ),
        (
            "lugus_resolve_passage",
            serde_json::json!({"passage_id":passage.id}),
            "Revenue & cash grew.",
        ),
    ] {
        let result = executor.execute(call(name, arguments)).await;
        assert!(result.success, "{name}: {}", result.content);
        assert!(result.content.contains(expected));
    }
    assert!(!executor.execute(call("lugus_create_passage", serde_json::json!({"request":{"representation_id":header.id,"start":0,"end":20,"expected_text":"Revenue & cash grew.","scope":{"workspace_id":"forged"}}}))).await.success);
    let mut wrong_run = call(
        "lugus_read_passage",
        serde_json::json!({"passage_id":passage.id}),
    );
    wrong_run.run_id = "forged".into();
    assert!(!executor.execute(wrong_run).await.success);
    let mut forged = scope();
    forged.workspace_id = "other".into();
    assert_eq!(
        app.read_passage(&forged, &passage.id)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    app.shutdown().await.unwrap();
}

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
struct HeldExtractor {
    entered: AtomicBool,
    release: AtomicBool,
    saw_cancel: AtomicBool,
}
impl HeldExtractor {
    fn new() -> Self {
        Self {
            entered: AtomicBool::new(false),
            release: AtomicBool::new(false),
            saw_cancel: AtomicBool::new(false),
        }
    }
}
impl TextExtractor for HeldExtractor {
    fn identity(&self) -> ExtractorIdentity {
        ExtractorIdentity::html_v1()
    }
    fn extract(
        &self,
        bytes: &[u8],
        media: &str,
        limits: &TextLimits,
        cancel: &AtomicBool,
    ) -> Result<ExtractedText> {
        self.entered.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.release.load(Ordering::Acquire) && Instant::now() < deadline {
            if cancel.load(Ordering::Acquire) {
                self.saw_cancel.store(true, Ordering::Release);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        extract_html(bytes, media, limits, cancel)
    }
}
async fn until(flag: &AtomicBool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !flag.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
async fn held_app(
    store: SqliteApplicationStore,
    fin: &std::path::Path,
    extractor: Arc<HeldExtractor>,
) -> Application {
    Application::start_with_text_options(
        vec![],
        Arc::new(SqliteRepositoryFactory::new(fin)),
        Box::new(store),
        Limits::default(),
        HostBounds::default(),
        Box::new(Ids(AtomicU64::new(1000))),
        TextPreparationOptions {
            extractor,
            max_concurrent_preparations: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn dropped_extraction_retains_capacity_allows_reads_and_shutdown_drains_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("fin");
    let db = dir.path().join("app");
    let mut store = open(&db, &fin, 1);
    let d = dataset(&mut store, &fin, "<p>Revenue &amp; cash grew.</p>");
    let extractor = Arc::new(HeldExtractor::new());
    let app = held_app(store, &fin, extractor.clone()).await;
    let worker = {
        let app = app.clone();
        let id = d.id.clone();
        tokio::spawn(async move { app.prepare_text(&scope(), &id).await })
    };
    until(&extractor.entered).await;
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(200),
            app.dataset_header(&scope(), &d.id)
        )
        .await
        .unwrap()
        .unwrap()
        .id,
        d.id
    );
    worker.abort();
    let _ = worker.await;
    until(&extractor.saw_cancel).await;
    assert_eq!(
        app.prepare_text(&scope(), &d.id).await.unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    let stop = {
        let app = app.clone();
        tokio::spawn(async move { app.shutdown().await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!stop.is_finished());
    assert_eq!(
        app.prepare_text(&scope(), &d.id).await.unwrap_err().kind,
        ErrorKind::Unavailable
    );
    extractor.release.store(true, Ordering::Release);
    stop.await.unwrap().unwrap();
    app.shutdown().await.unwrap();
    assert!(
        !app.read_document(&scope(), &d.id, 0, 100)
            .await
            .unwrap()
            .bytes
            .is_empty()
    );
    let sql = rusqlite::Connection::open(db).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM text_representations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn shutdown_during_admitted_save_does_not_block_async_executor() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("fin");
    let db = dir.path().join("app");
    let mut store = open(&db, &fin, 1);
    let d = dataset(&mut store, &fin, "<p>Revenue &amp; cash grew.</p>");
    let extractor = Arc::new(HeldExtractor::new());
    let app = held_app(store, &fin, extractor.clone()).await;
    let worker = {
        let app = app.clone();
        let id = d.id.clone();
        tokio::spawn(async move { app.prepare_text(&scope(), &id).await })
    };
    until(&extractor.entered).await;
    let sql = rusqlite::Connection::open(&db).unwrap();
    sql.execute_batch("BEGIN IMMEDIATE").unwrap();
    extractor.release.store(true, Ordering::Release);
    tokio::time::sleep(Duration::from_millis(30)).await;
    // A save holding the store is waiting for this independent SQLite write lock.
    assert!(
        tokio::time::timeout(
            Duration::from_millis(30),
            app.dataset_header(&scope(), &d.id)
        )
        .await
        .is_err()
    );
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        sql.execute_batch("COMMIT").unwrap();
    });
    let started = Instant::now();
    assert!(
        tokio::time::timeout(Duration::from_millis(40), app.shutdown())
            .await
            .is_err()
    );
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "shutdown blocked the current-thread executor"
    );
    release.join().unwrap();
    app.shutdown().await.unwrap();
    assert!(worker.await.unwrap().is_ok());
}

#[tokio::test]
async fn bounded_preparation_failure_and_unicode_selection_preserve_originals() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("fin");
    let db = dir.path().join("app");
    let mut store = open(&db, &fin, 1);
    let bad = dataset(
        &mut store,
        &fin,
        "<meta charset='utf-16'><p>secret original</p>",
    );
    let good = dataset(&mut store, &fin, "<p>é cash</p>");
    let app = application(store, &fin).await;
    assert_eq!(
        app.prepare_text(&scope(), &bad.id).await.unwrap_err().kind,
        ErrorKind::Unsupported
    );
    assert!(
        app.read_document(&scope(), &bad.id, 0, 100)
            .await
            .unwrap()
            .bytes
            .ends_with(b"</p>")
    );
    let sql = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM text_representations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let h = app.prepare_text(&scope(), &good.id).await.unwrap();
    assert_eq!(
        app.read_text(&scope(), &h.id, 1, 2).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        app.read_text(&scope(), &h.id, usize::MAX, 0)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        app.create_passage(
            &scope(),
            CreatePassageRequest {
                representation_id: h.id,
                start: 0,
                end: 7,
                expected_text: "stale".into()
            }
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::InvalidInput
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn preparation_input_larger_than_public_output_is_loaded_through_private_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("fin");
    let db = dir.path().join("app");
    let mut store = open(&db, &fin, 1);
    let d = dataset(
        &mut store,
        &fin,
        &format!("<script>{}</script><p>é cash</p>", "x".repeat(20000)),
    );
    drop(store);
    let limits = Limits {
        max_output_bytes: 4096,
        max_read_page_bytes: 4096,
        ..Limits::default()
    };
    let store = SqliteApplicationStore::open(
        &db,
        Box::new(lugus_financial::storage::SqliteRepository::open(&fin).unwrap()),
        limits.clone(),
        Box::new(SystemClock),
        Box::new(Ids(AtomicU64::new(100))),
    )
    .unwrap();
    let app = Application::start(
        vec![],
        Arc::new(SqliteRepositoryFactory::new(&fin)),
        Box::new(store),
        limits,
        HostBounds::default(),
        Box::new(Ids(AtomicU64::new(1000))),
    )
    .await
    .unwrap();
    let h = app.prepare_text(&scope(), &d.id).await.unwrap();
    assert_eq!(
        app.read_text(&scope(), &h.id, 0, 7).await.unwrap().text,
        "é cash"
    );
    app.shutdown().await.unwrap();
}
