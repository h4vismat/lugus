use lugus_financial::{
    domain::{CompanyId, ProviderIdentity, Validate},
    error::ErrorKind,
    instruments::*,
    market_data::InstrumentId,
    plugin::{Limits, Manifest, Plugin},
    storage::{
        SqliteRepository,
        bounded::{BoundedReadError, ReadLimits},
    },
};
use serde_json::json;
use std::path::PathBuf;

fn query() -> InstrumentLookup {
    InstrumentLookup {
        instrument: InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: "AAPL".into(),
        },
    }
}
fn provider() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "local".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    }
}
fn metadata() -> InstrumentMetadata {
    serde_json::from_value(json!({
        "instrument":{"namespace":"yahoo:symbol","value":"AAPL"},
        "issuer_name":"Apple Inc.","ticker":"AAPL",
        "exchange":{"namespace":"yahoo:exchange","value":"NMS"},"kind":"equity",
        "issuer_identifiers":[],"source_url":"https://finance.yahoo.com/quote/AAPL/",
        "source_checksum":"a".repeat(64),"retrieved_at":"2026-09-10T00:00:00Z"
    }))
    .unwrap()
}
fn limits() -> ReadLimits {
    ReadLimits {
        max_items: 1,
        max_bytes: 16_384,
    }
}
async fn start(mode: &str) -> Plugin {
    Plugin::start(
        Manifest {
            id: "instrument-fixture".into(),
            version: "1".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec!["instrument_plugin.py".into()],
        },
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "local".into(),
        json!({"mode":mode}),
        Limits::default(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn negotiates_and_dispatches_lookup_with_source_identity() {
    let mut plugin = start("ok").await;
    assert_eq!(plugin.capabilities().get("instrument_lookup"), Some(&1));
    assert_eq!(
        plugin
            .lookup_instrument(&query())
            .await
            .unwrap()
            .ticker
            .as_deref(),
        Some("AAPL")
    );
    plugin.close().await.unwrap();
}
#[tokio::test]
async fn unknown_nested_fields_and_mismatched_source_are_protocol_failures() {
    for mode in [
        "unknown",
        "instrument_unknown",
        "exchange_unknown",
        "issuer_unknown",
        "mismatch",
        "bad_name",
    ] {
        let mut plugin = start(mode).await;
        assert_eq!(
            plugin.lookup_instrument(&query()).await.unwrap_err().kind,
            ErrorKind::Protocol,
            "{mode}"
        );
        assert!(!plugin.is_running(), "{mode}");
    }
}
#[tokio::test]
async fn unsupported_lookup_version_never_dispatches_and_bad_query_does_not_close() {
    for mode in ["unsupported", "v2"] {
        let mut plugin = start(mode).await;
        assert_eq!(
            plugin.lookup_instrument(&query()).await.unwrap_err().kind,
            ErrorKind::Unsupported
        );
        assert!(plugin.is_running());
        plugin.close().await.unwrap();
    }
    let mut plugin = start("ok").await;
    let mut bad = query();
    bad.instrument.value = "a".repeat(129);
    assert_eq!(
        plugin.lookup_instrument(&bad).await.unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
    assert!(plugin.is_running());
    assert!(plugin.lookup_instrument(&query()).await.is_ok());
    plugin.close().await.unwrap();
}
#[test]
fn absent_optional_fields_remain_unknown_but_malformed_supplied_values_fail() {
    let mut value = serde_json::to_value(metadata()).unwrap();
    for field in ["issuer_name", "ticker", "exchange", "kind"] {
        value.as_object_mut().unwrap().remove(field);
    }
    let missing: InstrumentMetadata = serde_json::from_value(value).unwrap();
    missing.validate_for(&query()).unwrap();
    assert_eq!(missing.issuer_name, None);
    assert_eq!(missing.kind, None);
    assert_eq!(
        serde_json::to_value(missing).unwrap()["exchange"],
        json!(null)
    );
    for (field, bad) in [
        ("issuer_name", json!(42)),
        ("exchange", json!("NMS")),
        ("kind", json!("ETF")),
        ("ticker", json!([])),
    ] {
        let mut value = serde_json::to_value(metadata()).unwrap();
        value[field] = bad;
        assert!(
            serde_json::from_value::<InstrumentMetadata>(value).is_err(),
            "{field}"
        );
    }
}
#[test]
fn bounded_domain_fields_reject_controls_oversize_and_wrong_scope() {
    for bad in [
        "".to_owned(),
        "a".repeat(129),
        "é".repeat(65),
        "AA\u{7f}".into(),
        "A\nPL".into(),
    ] {
        let mut q = query();
        q.instrument.value = bad.clone();
        assert!(q.validate().is_err());
        let mut m = metadata();
        m.ticker = Some(bad);
        assert!(m.validate().is_err());
    }
    let mut m = metadata();
    m.issuer_name = Some("a".repeat(1025));
    assert!(m.validate().is_err());
    m = metadata();
    m.source_url = "a".repeat(4097);
    assert!(m.validate().is_err());
    m = metadata();
    m.source_checksum = "g".repeat(64);
    assert!(m.validate().is_err());
    m = metadata();
    m.issuer_identifiers = vec![
        CompanyId {
            namespace: "lei".into(),
            value: "id".into()
        };
        17
    ];
    assert!(m.validate().is_err());
    m = metadata();
    m.instrument.value = "MSFT".into();
    assert!(m.validate_for(&query()).is_err());
}
#[test]
fn exact_evidence_is_immutable_separate_for_every_retrieval_and_provider_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("evidence.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let first = repo
        .save_instrument_observation(&provider(), &query(), &metadata())
        .unwrap();
    let mut updated = metadata();
    updated.issuer_name = Some("New source name".into());
    let second = repo
        .save_instrument_observation(&provider(), &query(), &updated)
        .unwrap();
    assert_ne!(first.id, second.id);
    let third = repo
        .save_instrument_observation(&provider(), &query(), &updated)
        .unwrap();
    assert_ne!(second.id, third.id);
    let mut wrong = provider();
    wrong.plugin_version = "2".into();
    assert!(
        repo.bounded_instrument_observation(&wrong, first.id, limits())
            .is_err()
    );
    assert!(matches!(
        repo.bounded_instrument_observation(
            &provider(),
            first.id,
            ReadLimits {
                max_items: 1,
                max_bytes: 1
            }
        ),
        Err(BoundedReadError::LimitExceeded)
    ));
    drop(repo);
    let reopened = SqliteRepository::open(&path).unwrap();
    let restored = reopened
        .bounded_instrument_observation(&provider(), first.id, limits())
        .unwrap();
    assert_eq!(restored.metadata.issuer_name.as_deref(), Some("Apple Inc."));
    assert_eq!(restored.request, query());
    assert_eq!(restored.recorded_at, first.recorded_at);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert!(
        db.execute(
            "UPDATE instrument_observations SET payload='{}' WHERE id=?",
            [first.id]
        )
        .is_err()
    );
    assert!(
        db.execute("DELETE FROM instrument_observations WHERE id=?", [first.id])
            .is_err()
    );
}
#[test]
fn byte_preflight_precedes_deserialization_and_ignores_unrelated_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("evidence.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let first = repo
        .save_instrument_observation(&provider(), &query(), &metadata())
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("INSERT INTO instrument_observations(provider_id,provider,request,payload,recorded_at) SELECT provider_id,provider,request,?1,recorded_at FROM instrument_observations WHERE id=?2", rusqlite::params!["x".repeat(50_000), first.id]).unwrap();
    let oversized = db.last_insert_rowid();
    assert!(
        repo.bounded_instrument_observation(&provider(), first.id, limits())
            .is_ok()
    );
    assert!(matches!(
        repo.bounded_instrument_observation(&provider(), oversized, limits()),
        Err(BoundedReadError::LimitExceeded)
    ));
}
#[test]
fn version_four_migrates_without_rewriting_old_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v4.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../src/storage/schema.sql"))
        .unwrap();
    db.execute_batch(include_str!("../src/storage/market-v2.sql"))
        .unwrap();
    db.execute_batch(include_str!("../src/storage/catalog-v3.sql"))
        .unwrap();
    db.execute_batch(include_str!("../src/storage/selection-v4.sql"))
        .unwrap();
    db.execute("INSERT INTO providers VALUES('old','old evidence')", [])
        .unwrap();
    let identity: String = db
        .query_row("SELECT identity FROM repository_identity", [], |r| r.get(0))
        .unwrap();
    drop(db);
    let mut repo = SqliteRepository::open(&path).unwrap();
    assert_eq!(repo.repository_identity().unwrap(), identity);
    repo.save_instrument_observation(&provider(), &query(), &metadata())
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        db.query_row("SELECT identity FROM providers WHERE id='old'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "old evidence"
    );
}
#[test]
fn stored_provider_requires_complete_strict_json_and_all_fields_count_toward_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("evidence.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let first = repo
        .save_instrument_observation(&provider(), &query(), &metadata())
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let extra = format!("{} trailing", serde_json::to_string(&provider()).unwrap());
    db.execute("INSERT INTO instrument_observations(provider_id,provider,request,payload,recorded_at) SELECT provider_id,?1,request,payload,recorded_at FROM instrument_observations WHERE id=?2", rusqlite::params![extra, first.id]).unwrap();
    assert!(
        repo.bounded_instrument_observation(&provider(), db.last_insert_rowid(), limits())
            .is_err()
    );
    for field in ["provider", "request", "recorded_at"] {
        db.execute(&format!("INSERT INTO instrument_observations(provider_id,provider,request,payload,recorded_at) SELECT provider_id,{}, {},payload,{} FROM instrument_observations WHERE id=?2", if field == "provider" { "?1" } else { "provider" }, if field == "request" { "?1" } else { "request" }, if field == "recorded_at" { "?1" } else { "recorded_at" }), rusqlite::params!["x".repeat(50_000), first.id]).unwrap();
        assert!(
            matches!(
                repo.bounded_instrument_observation(&provider(), db.last_insert_rowid(), limits()),
                Err(BoundedReadError::LimitExceeded)
            ),
            "{field}"
        );
    }
}
