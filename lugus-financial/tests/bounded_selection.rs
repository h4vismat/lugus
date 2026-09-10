use lugus_financial::{
    domain::{ProviderIdentity, Query},
    error::ErrorKind,
    market_data::{PricePage, PriceQuery},
    storage::{
        Repository, SqliteRepository,
        bounded::{BoundedReadError, ReadLimits},
        market::MarketRepository,
    },
};
use serde_json::json;

fn provider() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "fixture".into(),
        plugin_id: "market".into(),
        plugin_version: "1".into(),
    }
}
fn query() -> PriceQuery {
    serde_json::from_value(json!({"instrument":{"namespace":"fixture:symbol","value":"IBM"},"start":"2024-01-01","end":"2024-12-31","cursor":null,"page_size":10})).unwrap()
}
fn page() -> PricePage {
    serde_json::from_value(json!({"items":[{
        "instrument":{"namespace":"fixture:symbol","value":"IBM"},"date":"2024-01-02",
        "open":"100.00","high":"102","low":"99","close":"101.000","volume":123,
        "adjusted_close":null,"currency":"USD","exchange_timezone":"America/New_York",
        "price_basis":"source_reported","precision":"decimal_source","source_url":"https://fixture.test/prices",
        "retrieved_at":"2024-02-01T00:00:00Z"}],"next_cursor":null,
        "coverage":{"first_date":"2024-01-02","last_date":"2024-01-02","completeness":"unverified"}})).unwrap()
}
fn limits() -> ReadLimits {
    ReadLimits {
        max_items: 10,
        max_bytes: 10_000,
    }
}

#[test]
fn exact_run_ignores_unrelated_payloads_and_checks_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("financial.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let id = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(id, &page()).unwrap();
    repo.finish_market_run(id, None).unwrap();
    let other = repo.start_market_run(&provider(), &query()).unwrap();
    let mut revised = page();
    revised.items[0].close = lugus_financial::domain::Decimal::new("102").unwrap();
    repo.save_prices_page(other, &revised).unwrap();
    repo.finish_market_run(other, None).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("UPDATE price_observations SET payload=? WHERE id IN (SELECT observation_id FROM market_run_observations WHERE run_id=?)", rusqlite::params!["x".repeat(20_000),other]).unwrap();
    let selected = repo.bounded_market_run(&provider(), id, limits()).unwrap();
    assert_eq!(selected.run.id, id);
    assert_eq!(selected.prices[0].value.close.as_str(), "101.000");
    assert_eq!(
        selected.prices[0].retrieval.retrieved_at.to_rfc3339(),
        "2024-02-01T00:00:00+00:00"
    );
    let identity = repo.repository_identity().unwrap();
    assert_eq!(selected.context.repository_id, identity);
    drop(repo);
    let repo = SqliteRepository::open(&path).unwrap();
    assert_eq!(repo.repository_identity().unwrap(), identity);
    let mut wrong = provider();
    wrong.plugin_version = "2".into();
    assert!(
        matches!(repo.bounded_market_run(&wrong,id,limits()), Err(BoundedReadError::Financial(e)) if e.kind == ErrorKind::NotFound)
    );
}

#[test]
fn payload_bytes_and_row_counts_are_checked_before_decode() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("financial.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let id = repo.start_market_run(&provider(), &query()).unwrap();
    let mut two = page();
    let mut second = two.items[0].clone();
    second.date = "2024-01-03".parse().unwrap();
    two.coverage.last_date = Some(second.date);
    two.items.push(second);
    repo.save_prices_page(id, &two).unwrap();
    assert!(matches!(
        repo.bounded_market_run(
            &provider(),
            id,
            ReadLimits {
                max_items: 1,
                ..limits()
            }
        ),
        Err(BoundedReadError::LimitExceeded)
    ));
    let sql = rusqlite::Connection::open(path).unwrap();
    sql.execute("UPDATE price_observations SET payload=?", ["é".repeat(800)])
        .unwrap();
    // Character count fits; UTF-8 bytes exceed the byte budget, and invalid JSON must never be decoded.
    assert!(matches!(
        repo.bounded_market_run(
            &provider(),
            id,
            ReadLimits {
                max_bytes: 2500,
                ..limits()
            }
        ),
        Err(BoundedReadError::LimitExceeded)
    ));
}

#[test]
fn financial_metadata_is_bounded_and_empty_runs_remain_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("financial.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let q: Query=serde_json::from_value(json!({"company":{"namespace":"sec:cik","value":"51143"},"filed_from":"2024-01-01","filed_to":"2024-12-31","forms":[],"cursor":null,"page_size":10})).unwrap();
    let id = repo.start_run(&provider(), &q, "facts").unwrap();
    repo.finish_run(id, None).unwrap();
    let data = repo
        .bounded_financial_run(&provider(), id, limits())
        .unwrap();
    assert!(data.facts.is_empty());
    assert!(data.filings.is_empty());
    assert_eq!(data.run.query.company.value, "51143");
    let sql = rusqlite::Connection::open(path).unwrap();
    sql.execute(
        "UPDATE runs SET query=? WHERE id=?",
        rusqlite::params!["x".repeat(20_000), id],
    )
    .unwrap();
    assert!(matches!(
        repo.bounded_financial_run(&provider(), id, limits()),
        Err(BoundedReadError::LimitExceeded)
    ));
    assert!(
        matches!(repo.bounded_financial_run(&provider(),id,ReadLimits{max_items:0,..limits()}),Err(BoundedReadError::Financial(e)) if e.kind==ErrorKind::InvalidRequest)
    );
}

#[test]
fn repository_identity_is_counted_before_context_materialization() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("financial.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let id = repo.start_market_run(&provider(), &query()).unwrap();
    let sql = rusqlite::Connection::open(path).unwrap();
    sql.execute(
        "UPDATE repository_identity SET identity=?",
        ["x".repeat(5000)],
    )
    .unwrap();
    assert!(matches!(
        repo.bounded_market_run(
            &provider(),
            id,
            ReadLimits {
                max_bytes: 1000,
                ..limits()
            }
        ),
        Err(BoundedReadError::LimitExceeded)
    ));
}

#[test]
fn resolution_history_is_bounded_without_replacing_identity_rules() {
    use lugus_financial::resolution::{
        ResolutionPage, SearchQuery, SearchRequest,
        catalog::{CatalogRepository, ResolutionOutcome},
    };
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("financial.db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let request = SearchRequest {
        query: SearchQuery::Identifier {
            identifier: lugus_financial::domain::CompanyId {
                namespace: "sec:ticker".into(),
                value: "IBM".into(),
            },
            exchange: None,
        },
        page_size: 10,
        cursor: None,
    };
    let page:ResolutionPage=serde_json::from_value(json!({"items":[{"identifier":{"namespace":"sec:cik","value":"0000051143"},"name":"IBM","aliases":[],"listings":[{"ticker":{"namespace":"sec:ticker","value":"IBM"},"exchange":null}],"source_url":"https://fixture.test/directory","source_checksum":"a".repeat(64),"retrieved_at":"2024-02-01T00:00:00Z","match_reasons":["exact_identifier"]}],"next_cursor":null,"snapshot":"one","coverage":"fixture"})).unwrap();
    let first = repo.start_resolution_run(&provider(), &request).unwrap();
    repo.save_resolution_page(first, &request, &page).unwrap();
    assert!(
        matches!(repo.bounded_resolution_outcome(first,limits()).unwrap(),ResolutionOutcome::Resolved{entry,..} if entry.candidate.name=="IBM")
    );
    let second = repo.start_resolution_run(&provider(), &request).unwrap();
    repo.save_resolution_page(second, &request, &page).unwrap();
    assert!(matches!(
        repo.bounded_resolution_outcome(
            second,
            ReadLimits {
                max_items: 1,
                ..limits()
            }
        ),
        Err(BoundedReadError::LimitExceeded)
    ));
    let sql = rusqlite::Connection::open(path).unwrap();
    sql.execute(
        "UPDATE catalog_observations SET payload=?",
        ["é".repeat(6000)],
    )
    .unwrap();
    assert!(matches!(
        repo.bounded_resolution_outcome(second, limits()),
        Err(BoundedReadError::LimitExceeded)
    ));
}
