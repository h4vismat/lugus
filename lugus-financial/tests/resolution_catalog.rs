use chrono::Utc;
use lugus_financial::{
    domain::{CompanyId, ProviderIdentity},
    resolution::{
        Candidate, Listing, MatchReason, ResolutionPage, SearchQuery, SearchRequest,
        catalog::{CatalogRepository, ResolutionOutcome},
    },
    storage::SqliteRepository,
};
fn id(namespace: &str, value: &str) -> CompanyId {
    CompanyId {
        namespace: namespace.into(),
        value: value.into(),
    }
}
fn provider() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "sec-main".into(),
        plugin_id: "sec-edgar".into(),
        plugin_version: "0.2.0".into(),
    }
}
fn request() -> SearchRequest {
    SearchRequest {
        query: SearchQuery::Identifier {
            identifier: id("sec:ticker", "IBM"),
            exchange: None,
        },
        page_size: 10,
        cursor: None,
    }
}
fn page(name: &str, next: Option<&str>) -> ResolutionPage {
    ResolutionPage {
        items: vec![Candidate {
            identifier: id("sec:cik", "0000051143"),
            name: name.into(),
            aliases: vec![],
            listings: vec![Listing {
                ticker: id("sec:ticker", "IBM"),
                exchange: Some(id("sec:exchange", "NYSE")),
            }],
            source_url: "https://www.sec.gov/files/company_tickers_exchange.json".into(),
            source_checksum: "a".repeat(64),
            retrieved_at: Utc::now(),
            match_reasons: vec![MatchReason::ExactIdentifier],
        }],
        next_cursor: next.map(str::to_owned),
        snapshot: "snapshot-1".into(),
        coverage: "directory".into(),
    }
}
#[test]
fn partial_is_not_resolved_and_completed_identity_survives_reopen_with_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut db = SqliteRepository::open(&path).unwrap();
    let req = request();
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("IBM", Some("next")))
        .unwrap();
    assert!(matches!(
        db.resolution_outcome(run).unwrap(),
        ResolutionOutcome::Incomplete { .. }
    ));
    db.fail_resolution_run(run, "network failure").unwrap();
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("International Business Machines", None))
        .unwrap();
    let company = match db.resolution_outcome(run).unwrap() {
        ResolutionOutcome::Resolved { entry, .. } => entry.company,
        _ => panic!("expected unique completed ticker"),
    };
    drop(db);
    let db = SqliteRepository::open(path).unwrap();
    let entries = db.search_catalog(&req.query).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].company, company);
    assert_eq!(db.catalog_history(company).unwrap().len(), 2);
}
#[test]
fn snapshot_mismatch_rolls_back_and_failed_refresh_retains_evidence() {
    let mut db = SqliteRepository::open(":memory:").unwrap();
    let req = request();
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("IBM", Some("next")))
        .unwrap();
    let mut continuation = req.clone();
    continuation.cursor = Some("next".into());
    let mut changed = page("Changed", None);
    changed.snapshot = "another".into();
    assert!(
        db.save_resolution_page(run, &continuation, &changed)
            .is_err()
    );
    assert_eq!(
        db.search_catalog(&req.query).unwrap()[0].candidate.name,
        "IBM"
    );
}
#[test]
fn names_never_merge_ticker_ambiguity_and_later_conflict_are_explicit() {
    let mut db = SqliteRepository::open(":memory:").unwrap();
    let req = request();
    let mut both = page("Same Name", None);
    let mut other = both.items[0].clone();
    other.identifier = id("sec:cik", "0000000001");
    both.items.push(other);
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &both).unwrap();
    assert!(
        matches!(db.resolution_outcome(run).unwrap(),ResolutionOutcome::Candidates{items,..} if items.len()==2)
    );
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("IBM", None))
        .unwrap();
    assert!(
        matches!(db.resolution_outcome(run).unwrap(),ResolutionOutcome::IdentityConflict{items,..} if items.len()==2)
    );
}
#[test]
fn repeated_records_share_observation_but_retain_each_retrieval_and_empty_does_not_delete() {
    let mut db = SqliteRepository::open(":memory:").unwrap();
    let req = request();
    for _ in 0..2 {
        let run = db.start_resolution_run(&provider(), &req).unwrap();
        db.save_resolution_page(run, &req, &page("IBM", None))
            .unwrap();
    }
    let company = db.search_catalog(&req.query).unwrap()[0].company;
    let history = db.catalog_history(company).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].observation_id, history[1].observation_id);
    assert_ne!(history[0].run_id, history[1].run_id);
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    let mut empty = page("unused", None);
    empty.items.clear();
    db.save_resolution_page(run, &req, &empty).unwrap();
    assert!(matches!(
        db.resolution_outcome(run).unwrap(),
        ResolutionOutcome::NoMatch { .. }
    ));
    assert_eq!(db.search_catalog(&req.query).unwrap().len(), 1);
}
#[test]
fn cursor_replay_duplicate_entity_and_post_completion_writes_are_rejected() {
    let mut db = SqliteRepository::open(":memory:").unwrap();
    let req = request();
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("IBM", Some("next")))
        .unwrap();
    assert!(
        db.save_resolution_page(run, &req, &page("IBM", None))
            .is_err()
    );
    let mut continuation = req.clone();
    continuation.cursor = Some("next".into());
    assert!(
        db.save_resolution_page(run, &continuation, &page("IBM", None))
            .is_err()
    );
    let mut final_page = page("unused", None);
    final_page.items.clear();
    db.save_resolution_page(run, &continuation, &final_page)
        .unwrap();
    assert!(
        db.save_resolution_page(run, &continuation, &final_page)
            .is_err()
    );
    assert!(db.fail_resolution_run(run, "late").is_err());
}
#[test]
fn version_two_migration_is_additive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../src/storage/schema.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../src/storage/market-v2.sql"))
        .unwrap();
    connection.execute_batch("CREATE TABLE frozen_review_marker(value TEXT); INSERT INTO frozen_review_marker VALUES('unchanged');").unwrap();
    drop(connection);
    let mut db = SqliteRepository::open(&path).unwrap();
    let req = request();
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("IBM", None))
        .unwrap();
    drop(db);
    let connection = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM frozen_review_marker", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "unchanged"
    );
}

#[test]
fn offline_matching_recomputes_reasons_and_sorts_exact_names_first() {
    let mut db = SqliteRepository::open(":memory:").unwrap();
    let req = SearchRequest {
        query: SearchQuery::Name { text: "IBM".into() },
        page_size: 10,
        cursor: None,
    };
    let mut matches = page("IBM Research", None);
    matches.items[0].match_reasons = vec![MatchReason::NameSubstring];
    let mut exact = matches.items[0].clone();
    exact.name = "IBM".into();
    exact.identifier = id("sec:cik", "0000000001");
    exact.match_reasons = vec![MatchReason::ExactName];
    matches.items.push(exact);
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &matches).unwrap();
    assert!(matches!(
        db.resolution_outcome(run).unwrap(),
        ResolutionOutcome::Candidates { .. }
    ));
    let found = db.search_catalog(&req.query).unwrap();
    assert_eq!(found[0].candidate.name, "IBM");
    let found = db.search_catalog(&request().query).unwrap();
    assert!(
        found
            .iter()
            .all(|e| e.candidate.match_reasons == vec![MatchReason::ExactIdentifier])
    );
}
#[test]
fn unknown_entity_namespaces_are_provider_scoped_but_versions_preserve_identity() {
    let mut db = SqliteRepository::open(":memory:").unwrap();
    let mut req = request();
    req.query = SearchQuery::Identifier {
        identifier: id("custom:entity", "123"),
        exchange: None,
    };
    let mut candidate = page("IBM", None);
    candidate.items[0].identifier = id("custom:entity", "123");
    for instance in ["first", "second"] {
        let mut source = provider();
        source.instance_id = instance.into();
        let run = db.start_resolution_run(&source, &req).unwrap();
        db.save_resolution_page(run, &req, &candidate).unwrap();
    }
    let companies = db.search_catalog(&req.query).unwrap();
    assert_eq!(companies.len(), 2);
    let mut source = provider();
    source.instance_id = "first".into();
    source.plugin_version = "next-version".into();
    let run = db.start_resolution_run(&source, &req).unwrap();
    db.save_resolution_page(run, &req, &candidate).unwrap();
    assert_eq!(db.search_catalog(&req.query).unwrap().len(), 2);
    assert!(matches!(
        db.resolution_outcome(run).unwrap(),
        ResolutionOutcome::Resolved { .. }
    ));
    let history = db.catalog_history(companies[0].company).unwrap();
    assert_eq!(history.len(), 2);
    assert_ne!(
        history[0].provider.plugin_version,
        history[1].provider.plugin_version
    );
}

#[test]
fn explicit_offline_choice_survives_reopen_without_rewriting_ambiguous_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chosen.db");
    let mut db = SqliteRepository::open(&path).unwrap();
    let req = SearchRequest {
        query: SearchQuery::Name { text: "IBM".into() },
        page_size: 10,
        cursor: None,
    };
    let mut result = page("IBM", None);
    result.items[0].match_reasons = vec![MatchReason::ExactName];
    let mut other = result.items[0].clone();
    other.identifier = id("sec:cik", "0000000001");
    result.items.push(other);
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &result).unwrap();
    let entries = db.search_catalog(&req.query).unwrap();
    let choice = db.select_candidate(run, entries[0].observation_id).unwrap();
    assert_eq!(choice.entry.company, entries[0].company);
    assert!(matches!(
        choice.run_status,
        lugus_financial::storage::RunStatus::Complete
    ));
    assert!(matches!(
        db.resolution_outcome(run).unwrap(),
        ResolutionOutcome::Candidates { .. }
    ));
    drop(db);
    let db = SqliteRepository::open(path).unwrap();
    let reopened = db.catalog_selection(choice.id).unwrap();
    assert_eq!(reopened.entry.observation_id, choice.entry.observation_id);
    assert_eq!(reopened.selected_at, choice.selected_at);
    assert_eq!(reopened.entry.provider.instance_id, "sec-main");
}
#[test]
fn choice_validates_run_membership_and_freezes_partial_search_status() {
    let mut db = SqliteRepository::open(":memory:").unwrap();
    let req = request();
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("IBM", Some("next")))
        .unwrap();
    let entry = db.search_catalog(&req.query).unwrap().remove(0);
    let empty_run = db.start_resolution_run(&provider(), &req).unwrap();
    assert!(
        db.select_candidate(empty_run, entry.observation_id)
            .is_err()
    );
    assert!(db.select_candidate(run, 99999).is_err());
    assert!(db.select_candidate(99999, entry.observation_id).is_err());
    let choice = db.select_candidate(run, entry.observation_id).unwrap();
    assert!(matches!(
        choice.run_status,
        lugus_financial::storage::RunStatus::Running
    ));
    assert_eq!(choice.snapshot.as_deref(), Some("snapshot-1"));
    db.fail_resolution_run(run, "unavailable").unwrap();
    let frozen = db.catalog_selection(choice.id).unwrap();
    assert!(matches!(
        frozen.run_status,
        lugus_financial::storage::RunStatus::Running
    ));
    assert!(frozen.error.is_none());
    assert!(matches!(
        db.resolution_outcome(run).unwrap(),
        ResolutionOutcome::Incomplete { .. }
    ));
    let after_failure = db.select_candidate(run, entry.observation_id).unwrap();
    assert!(matches!(
        after_failure.run_status,
        lugus_financial::storage::RunStatus::Failed
    ));
    assert_eq!(after_failure.error.as_deref(), Some("unavailable"));
    assert_ne!(after_failure.id, choice.id);
}

#[test]
fn typed_failure_and_retry_hint_survive_reopen_and_explicit_choice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("failure.db");
    let mut db = SqliteRepository::open(&path).unwrap();
    let req = request();
    let run = db.start_resolution_run(&provider(), &req).unwrap();
    db.save_resolution_page(run, &req, &page("IBM", Some("next")))
        .unwrap();
    let mut error = lugus_financial::error::Error::new(
        lugus_financial::error::ErrorKind::RateLimited,
        "slow down",
    );
    error.retry_after_seconds = Some(30);
    db.fail_resolution_run_with_error(run, &error).unwrap();
    drop(db);
    let mut db = SqliteRepository::open(path).unwrap();
    let entry = match db.resolution_outcome(run).unwrap() {
        ResolutionOutcome::Incomplete {
            items,
            failure: Some(failure),
            error,
            ..
        } => {
            assert_eq!(failure.kind, lugus_financial::error::ErrorKind::RateLimited);
            assert_eq!(failure.retry_after_seconds, Some(30));
            assert_eq!(error.as_deref(), Some("slow down"));
            items.into_iter().next().unwrap()
        }
        _ => panic!("expected typed failure"),
    };
    let choice = db.select_candidate(run, entry.observation_id).unwrap();
    assert_eq!(choice.failure.unwrap().retry_after_seconds, Some(30));
}
