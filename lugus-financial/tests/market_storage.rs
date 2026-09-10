use async_trait::async_trait;
use chrono::Utc;
use lugus_financial::{
    application::market::ingest_prices,
    capabilities::{MarketDataProvider, Provider},
    domain::{Decimal, ProviderIdentity},
    error::*,
    market_data::*,
    storage::{RunStatus, SqliteRepository, market::MarketRepository},
};
fn query() -> PriceQuery {
    PriceQuery {
        instrument: InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: "AAPL".into(),
        },
        start: "2024-01-01".parse().unwrap(),
        end: "2024-12-31".parse().unwrap(),
        cursor: None,
        page_size: 2,
    }
}
fn identity() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "local".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    }
}
fn bar(day: &str) -> PriceBar {
    PriceBar {
        instrument: query().instrument,
        date: day.parse().unwrap(),
        open: Decimal::new("2").unwrap(),
        high: Decimal::new("3").unwrap(),
        low: Decimal::new("1").unwrap(),
        close: Decimal::new("2").unwrap(),
        volume: 12,
        adjusted_close: None,
        currency: "USD".into(),
        exchange_timezone: "America/New_York".into(),
        price_basis: PriceBasis::SourceReported,
        precision: PricePrecision::BinaryFloatSource,
        source_url: "https://example.test".into(),
        retrieved_at: Utc::now(),
    }
}
fn page() -> PricePage {
    PricePage {
        items: vec![bar("2024-01-02"), bar("2024-01-03")],
        next_cursor: None,
        coverage: PriceCoverage {
            first_date: Some("2024-01-02".parse().unwrap()),
            last_date: Some("2024-01-03".parse().unwrap()),
            completeness: Completeness::Unverified,
        },
    }
}
#[test]
fn revisions_retrieval_history_and_offline_states() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let mut p = page();
    let mut ids = vec![];
    for i in 0..3 {
        if i == 2 {
            p.items[0].volume += 1;
        }
        p.items[0].retrieved_at = Utc::now();
        let run = repo.start_market_run(&identity(), &query()).unwrap();
        repo.save_prices_page(run, &p).unwrap();
        repo.finish_market_run(run, None).unwrap();
        ids.push(run);
    }
    let a = repo.market_run_observations(ids[0]).unwrap();
    let b = repo.market_run_observations(ids[1]).unwrap();
    assert_eq!(a[0].observation_id, b[0].observation_id);
    assert!(b[0].retrieved_at >= a[0].retrieved_at);
    repo.start_market_run(&identity(), &query()).unwrap();
    drop(repo);
    let repo = SqliteRepository::open(&path).unwrap();
    let s = repo.market_snapshot(&identity(), &query()).unwrap();
    assert_eq!(s.prices.len(), 3);
    assert_eq!(s.runs.len(), 4);
    assert_eq!(s.runs[3].status, RunStatus::Running);
    assert_eq!(
        s.runs[0].coverage.as_ref().unwrap().completeness,
        Completeness::Unverified
    );
}
#[test]
fn atomic_sql_failure_and_validation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let run = repo.start_market_run(&identity(), &query()).unwrap();
    let c = rusqlite::Connection::open(path).unwrap();
    c.execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON price_observations WHEN (SELECT COUNT(*) FROM price_observations)=1 BEGIN SELECT RAISE(ABORT,'failure'); END;").unwrap();
    assert!(repo.save_prices_page(run, &page()).is_err());
    let s = repo.market_snapshot(&identity(), &query()).unwrap();
    assert!(s.prices.is_empty());
    assert!(s.runs[0].coverage.is_none());
    assert!(repo.market_run_observations(run).unwrap().is_empty());
    c.execute_batch("DROP TRIGGER reject_second;").unwrap();
    for mode in 0..4 {
        let mut p = page();
        match mode {
            0 => p.items[1].instrument.value = "other".into(),
            1 => p.items[1].date = "2025-01-01".parse().unwrap(),
            2 => p.items.swap(0, 1),
            _ => p.coverage.first_date = Some("2024-01-01".parse().unwrap()),
        };
        assert!(repo.save_prices_page(run, &p).is_err());
        assert!(
            repo.market_snapshot(&identity(), &query())
                .unwrap()
                .prices
                .is_empty()
        );
    }
    repo.save_prices_page(run, &page()).unwrap();
    assert!(repo.save_prices_page(run, &page()).is_err());
}
struct Fixture {
    id: ProviderIdentity,
    fail: bool,
    cursors: Vec<Option<String>>,
}
impl Provider for Fixture {
    fn identity(&self) -> &ProviderIdentity {
        &self.id
    }
}
#[async_trait]
impl MarketDataProvider for Fixture {
    async fn fetch_prices(&mut self, q: &PriceQuery) -> Result<PricePage> {
        self.cursors.push(q.cursor.clone());
        let mut p = page();
        if q.cursor.is_none() {
            p.items.truncate(1);
            p.next_cursor = Some("next".into());
        } else if self.fail {
            return Err(Error::new(ErrorKind::Unavailable, "failure"));
        } else {
            p.items.remove(0);
        }
        Ok(p)
    }
}
#[tokio::test]
async fn failed_page_and_restart() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let mut provider = Fixture {
        id: identity(),
        fail: true,
        cursors: vec![],
    };
    assert!(
        ingest_prices(&mut repo, &mut provider, &query())
            .await
            .is_err()
    );
    let s = repo.market_snapshot(&identity(), &query()).unwrap();
    assert_eq!(s.prices.len(), 1);
    assert_eq!(s.runs[0].status, RunStatus::Failed);
    assert_eq!(s.runs[0].cursor.as_deref(), Some("next"));
    provider.fail = false;
    let mut q = query();
    q.cursor = Some("stale".into());
    ingest_prices(&mut repo, &mut provider, &q).await.unwrap();
    assert_eq!(provider.cursors[2], None);
    assert_eq!(
        repo.market_snapshot(&identity(), &q).unwrap().prices.len(),
        2
    );
}
#[test]
fn stable_coverage_order_and_empty() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let run = repo.start_market_run(&identity(), &query()).unwrap();
    let mut p = page();
    p.items.truncate(1);
    p.next_cursor = Some("next".into());
    repo.save_prices_page(run, &p).unwrap();
    assert!(repo.finish_market_run(run, None).is_err());
    let mut next = page();
    next.items.remove(0);
    next.coverage.completeness = Completeness::Complete;
    assert!(repo.save_prices_page(run, &next).is_err());
    next.coverage.completeness = Completeness::Unverified;
    next.items[0].date = p.items[0].date;
    assert!(repo.save_prices_page(run, &next).is_err());
    let run = repo.start_market_run(&identity(), &query()).unwrap();
    let empty = PricePage {
        items: vec![],
        next_cursor: None,
        coverage: PriceCoverage {
            first_date: None,
            last_date: None,
            completeness: Completeness::Unverified,
        },
    };
    repo.save_prices_page(run, &empty).unwrap();
    repo.finish_market_run(run, None).unwrap();
}
#[test]
fn real_version_one_migration_preserves_sec_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(include_str!("../src/storage/schema.sql"))
        .unwrap();
    c.execute_batch("INSERT INTO providers VALUES('sec','{}');INSERT INTO companies VALUES('sec:cik','1');INSERT INTO runs VALUES(7,'sec','{}','facts','complete',NULL,NULL,NULL,'then','then');INSERT INTO observations VALUES(9,'sec','fact','hash','payload','metric');INSERT INTO run_observations VALUES(7,9,'then');INSERT INTO document_content VALUES('checksum',X'01');INSERT INTO document_observations VALUES(3,'sec','checksum','url','text/plain','then');").unwrap();
    drop(c);
    let mut repo = SqliteRepository::open(&path).unwrap();
    let run = repo.start_market_run(&identity(), &query()).unwrap();
    repo.save_prices_page(run, &page()).unwrap();
    let c = rusqlite::Connection::open(path).unwrap();
    for table in [
        "companies",
        "runs",
        "observations",
        "run_observations",
        "document_content",
        "document_observations",
    ] {
        let count: i64 = c
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
    let payload: String = c
        .query_row("SELECT payload FROM observations WHERE id=9", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(payload, "payload");
    let version: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 5);
}

struct Malicious {
    id: ProviderIdentity,
    mode: u8,
}
impl Provider for Malicious {
    fn identity(&self) -> &ProviderIdentity {
        &self.id
    }
}
#[async_trait]
impl MarketDataProvider for Malicious {
    async fn fetch_prices(&mut self, q: &PriceQuery) -> Result<PricePage> {
        let mut p = page();
        p.coverage.last_date = Some("2024-01-04".parse().unwrap());
        if q.cursor.is_none() {
            p.items.truncate(1);
            p.next_cursor = Some("next".into());
        } else {
            p.items.remove(0);
            p.next_cursor = Some("next".into());
            match self.mode {
                1 => {
                    p.coverage.completeness = Completeness::Complete;
                    p.next_cursor = Some("another".into());
                }
                2 => {
                    p.items[0].date = "2024-01-02".parse().unwrap();
                    p.next_cursor = Some("another".into());
                }
                _ => {}
            }
        }
        Ok(p)
    }
}
#[tokio::test]
async fn application_rejects_repeated_cursors_changed_coverage_and_nonascending_pages() {
    for mode in 0..3 {
        let mut repo = SqliteRepository::open(":memory:").unwrap();
        let mut provider = Malicious {
            id: identity(),
            mode,
        };
        let error = ingest_prices(&mut repo, &mut provider, &query())
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Protocol);
        let s = repo.market_snapshot(&identity(), &query()).unwrap();
        assert_eq!(s.prices.len(), 1);
        assert_eq!(s.runs[0].status, RunStatus::Failed);
        assert_eq!(s.runs[0].cursor.as_deref(), Some("next"));
        assert_eq!(
            s.runs[0].coverage.as_ref().unwrap().completeness,
            Completeness::Unverified
        );
    }
}
#[test]
fn provider_instrument_and_date_isolation() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let run = repo.start_market_run(&identity(), &query()).unwrap();
    repo.save_prices_page(run, &page()).unwrap();
    let mut other = identity();
    other.plugin_version = "2".into();
    let s = repo.market_snapshot(&other, &query()).unwrap();
    assert!(s.runs.is_empty());
    assert!(s.prices.is_empty());
    let mut q = query();
    q.instrument.value = "MSFT".into();
    assert!(
        repo.market_snapshot(&identity(), &q)
            .unwrap()
            .prices
            .is_empty()
    );
    q = query();
    q.start = "2024-01-03".parse().unwrap();
    assert_eq!(
        repo.market_snapshot(&identity(), &q).unwrap().prices.len(),
        1
    );
    q.start = "2024-02-01".parse().unwrap();
    assert!(
        repo.market_snapshot(&identity(), &q)
            .unwrap()
            .prices
            .is_empty()
    );
}
