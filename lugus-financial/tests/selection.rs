use lugus_financial::{domain::Decimal, selection::decimal_key};
#[test]
fn decimal_equivalence_is_exact() {
    for (a, b) in [("90", "90.00"), ("-0.00", "0"), ("0001.200", "1.2")] {
        assert_eq!(
            decimal_key(&Decimal::new(a).unwrap()),
            decimal_key(&Decimal::new(b).unwrap())
        );
    }
    assert_ne!(
        decimal_key(&Decimal::new("9007199254740992").unwrap()),
        decimal_key(&Decimal::new("9007199254740993").unwrap())
    );
}
use lugus_financial::{
    domain::*,
    market_data::*,
    selection::*,
    storage::{Repository, RunStatus, SqliteRepository, market::MarketRepository},
};
fn provider() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "local".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    }
}
fn query() -> PriceQuery {
    PriceQuery {
        instrument: InstrumentId {
            namespace: "fixture:symbol".into(),
            value: "IBM".into(),
        },
        start: "2024-01-01".parse().unwrap(),
        end: "2024-12-31".parse().unwrap(),
        cursor: None,
        page_size: 100,
    }
}
fn scope() -> Query {
    Query {
        company: CompanyId {
            namespace: "sec:cik".into(),
            value: "51143".into(),
        },
        filed_from: query().start,
        filed_to: query().end,
        forms: vec![],
        cursor: None,
        page_size: 100,
    }
}
fn metric() -> MetricQuery {
    MetricQuery {
        scope: scope(),
        namespace: "us-gaap".into(),
        concept: "Assets".into(),
        unit: "USD".into(),
        periods: PeriodSelection::LatestInstant,
    }
}
fn bar(value: &str, day: &str) -> PriceBar {
    PriceBar {
        instrument: query().instrument,
        date: day.parse().unwrap(),
        open: Decimal::new(value).unwrap(),
        high: Decimal::new(value).unwrap(),
        low: Decimal::new(value).unwrap(),
        close: Decimal::new(value).unwrap(),
        volume: 10,
        adjusted_close: None,
        currency: "USD".into(),
        exchange_timezone: "America/New_York".into(),
        price_basis: PriceBasis::SourceReported,
        precision: PricePrecision::DecimalSource,
        source_url: "https://fixture.test/prices".into(),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
    }
}
fn page(items: Vec<PriceBar>) -> PricePage {
    PricePage {
        coverage: PriceCoverage {
            first_date: items.first().map(|b| b.date),
            last_date: items.last().map(|b| b.date),
            completeness: Completeness::Unverified,
        },
        items,
        next_cursor: None,
    }
}
fn fact(value: &str, filed: &str, period: Period) -> Fact {
    Fact {
        company: scope().company,
        namespace: "us-gaap".into(),
        concept: "Assets".into(),
        label: None,
        value: Decimal::new(value).unwrap(),
        unit: "USD".into(),
        period,
        filing_id: format!("filing-{value}-{filed}"),
        form: "10-K".into(),
        filed: filed.parse().unwrap(),
        fiscal_year: Some(2024),
        fiscal_period: Some("FY".into()),
        source_url: "https://fixture.test/facts".into(),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
    }
}
fn instant(day: &str) -> Period {
    Period::Instant {
        date: day.parse().unwrap(),
    }
}
fn save_facts(repo: &mut SqliteRepository, facts: Vec<Fact>) -> i64 {
    let run = repo.start_run(&provider(), &scope(), "facts").unwrap();
    repo.save_facts_page(
        run,
        &Page {
            items: facts,
            next_cursor: None,
        },
        &|_| None,
    )
    .unwrap();
    repo.finish_run(run, None).unwrap();
    run
}
#[test]
fn coherent_latest_start_empty_partial_and_pinned_history() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let a = repo.start_market_run(&provider(), &query()).unwrap();
    let b = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(
        b,
        &page(vec![bar("98", "2024-01-02"), bar("99", "2024-01-03")]),
    )
    .unwrap();
    repo.finish_market_run(b, None).unwrap();
    repo.save_prices_page(
        a,
        &page(vec![bar("100", "2024-01-02"), bar("101", "2024-01-03")]),
    )
    .unwrap();
    repo.finish_market_run(a, None).unwrap();
    let data = repo.market_runs(&provider()).unwrap();
    let selected = select_daily(&provider(), &query(), PriceSeries::Close, &data, None).unwrap();
    assert_eq!(selected.manifest.selected_run.as_ref().unwrap().run_id, b);
    assert_eq!(selected.latest_close().unwrap().value.close.as_str(), "99");
    let frozen = serde_json::to_string(&selected.manifest).unwrap();
    let failed = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(failed, &page(vec![bar("1", "2024-01-02")]))
        .unwrap();
    repo.finish_market_run(
        failed,
        Some(&lugus_financial::error::Error::new(
            lugus_financial::error::ErrorKind::Unavailable,
            "offline",
        )),
    )
    .unwrap();
    let data = repo.market_runs(&provider()).unwrap();
    let current = select_daily(&provider(), &query(), PriceSeries::Close, &data, None).unwrap();
    assert_eq!(current.manifest.selected_run.unwrap().run_id, b);
    assert_eq!(
        current.manifest.available_runs.last().unwrap().status,
        RunStatus::Failed
    );
    let historical = select_daily(
        &provider(),
        &query(),
        PriceSeries::Close,
        &data,
        Some(failed),
    )
    .unwrap();
    assert_eq!(historical.prices.len(), 1);
    assert!(
        historical
            .manifest
            .limitations
            .iter()
            .any(|s| s.contains("partial"))
    );
    let empty = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(empty, &page(vec![])).unwrap();
    repo.finish_market_run(empty, None).unwrap();
    let data = repo.market_runs(&provider()).unwrap();
    let current = select_daily(&provider(), &query(), PriceSeries::Close, &data, None).unwrap();
    assert_eq!(current.manifest.selected_run.unwrap().run_id, empty);
    assert!(current.prices.is_empty());
    assert_eq!(frozen, serde_json::to_string(&selected.manifest).unwrap());
    assert_eq!(
        select_daily(&provider(), &query(), PriceSeries::Close, &data, Some(a))
            .unwrap()
            .latest_close()
            .unwrap()
            .value
            .close
            .as_str(),
        "101"
    );
}
#[test]
fn containment_scope_provider_and_no_complete_dataset() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let mut narrow = query();
    narrow.start = "2024-06-01".parse().unwrap();
    let id = repo.start_market_run(&provider(), &narrow).unwrap();
    repo.save_prices_page(id, &page(vec![])).unwrap();
    repo.finish_market_run(id, None).unwrap();
    let running = repo.start_market_run(&provider(), &query()).unwrap();
    let data = repo.market_runs(&provider()).unwrap();
    let s = select_daily(&provider(), &query(), PriceSeries::Close, &data, None).unwrap();
    assert!(s.manifest.selected_run.is_none());
    assert_eq!(s.manifest.available_runs.len(), 1);
    assert_eq!(s.manifest.available_runs[0].run_id, running);
    assert!(select_daily(&provider(), &query(), PriceSeries::Close, &data, Some(id)).is_err());
    let mut p = provider();
    p.plugin_version = "2".into();
    assert!(
        select_daily(&p, &query(), PriceSeries::Close, &data, None)
            .unwrap()
            .manifest
            .available_runs
            .is_empty()
    );
    let mut all = scope();
    let mut restricted = scope();
    restricted.forms = vec!["10-K".into()];
    assert!(financial_contains(&all, &restricted));
    assert!(!financial_contains(&restricted, &all));
    all.forms = vec!["10-K".into(), "10-Q".into()];
    assert!(financial_contains(&all, &restricted));
    restricted.forms.push("10-K/A".into());
    assert!(!financial_contains(&all, &restricted));
}
#[test]
fn market_conflicts_gaps_and_permutation_invariance() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let id = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(
        id,
        &page(vec![bar("1", "2024-01-02"), bar("2", "2024-01-03")]),
    )
    .unwrap();
    repo.finish_market_run(id, None).unwrap();
    let mut data = repo.market_runs(&provider()).unwrap();
    let s = select_daily(
        &provider(),
        &query(),
        PriceSeries::AdjustedClose,
        &data,
        None,
    )
    .unwrap();
    assert!(s.values.iter().all(Option::is_none));
    data[0].prices.reverse();
    let reversed = select_daily(
        &provider(),
        &query(),
        PriceSeries::AdjustedClose,
        &data,
        None,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(s).unwrap(),
        serde_json::to_value(reversed).unwrap()
    );
    data[0].prices[0].value.currency = "EUR".into();
    assert!(
        !select_daily(&provider(), &query(), PriceSeries::Close, &data, None)
            .unwrap()
            .conflicts
            .is_empty()
    );
    data[0].prices[0].value.currency = "USD".into();
    let duplicate = data[0].prices[0].clone();
    data[0].prices.push(duplicate);
    let conflict = select_daily(&provider(), &query(), PriceSeries::Close, &data, None).unwrap();
    assert!(conflict.latest_close().is_none());
    assert!(conflict.values.is_empty());
}
#[test]
fn exact_period_latest_disclosure_conflict_and_equal_support() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    save_facts(
        &mut repo,
        vec![
            fact("100", "2024-01-01", instant("2023-12-31")),
            fact("90", "2024-02-01", instant("2023-12-31")),
            fact("90.00", "2024-02-01", instant("2023-12-31")),
        ],
    );
    let mut data = repo.financial_runs(&provider()).unwrap();
    let s = select_facts(&provider(), &metric(), &data, None).unwrap();
    assert_eq!(s.groups[0].value.as_ref().unwrap().as_str(), "90");
    assert_eq!(s.groups[0].candidates.len(), 2);
    data[0].facts.reverse();
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        serde_json::to_value(select_facts(&provider(), &metric(), &data, None).unwrap()).unwrap()
    );
    save_facts(
        &mut repo,
        vec![
            fact("90", "2024-02-01", instant("2023-12-31")),
            fact("1", "2024-04-01", instant("2024-03-31")),
            fact("2", "2024-04-01", instant("2024-03-31")),
        ],
    );
    let data = repo.financial_runs(&provider()).unwrap();
    let s = select_facts(&provider(), &metric(), &data, None).unwrap();
    assert_eq!(s.groups.len(), 1);
    assert_eq!(s.groups[0].period, instant("2024-03-31"));
    assert!(s.groups[0].conflict.is_some());
    assert!(s.groups[0].value.is_none());
    let annual = Period::Duration {
        start: "2023-01-01".parse().unwrap(),
        end: "2023-12-31".parse().unwrap(),
    };
    let nine = Period::Duration {
        start: "2023-04-01".parse().unwrap(),
        end: "2023-12-31".parse().unwrap(),
    };
    save_facts(
        &mut repo,
        vec![
            fact("100", "2024-01-01", annual),
            fact("80", "2024-01-01", nine),
        ],
    );
    let mut q = metric();
    q.periods = PeriodSelection::Durations;
    let s = select_facts(
        &provider(),
        &q,
        &repo.financial_runs(&provider()).unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(s.groups.len(), 2);
}
#[test]
fn chronology_reopen_and_run_specific_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let f = fact("1", "2024-01-01", instant("2023-12-31"));
    let a = save_facts(&mut repo, vec![f.clone()]);
    let b = repo.start_market_run(&provider(), &query()).unwrap();
    let mut newer = f;
    newer.retrieved_at = "2025-01-01T00:00:00Z".parse().unwrap();
    let c = save_facts(&mut repo, vec![newer]);
    let financial = repo.financial_runs(&provider()).unwrap();
    let market = repo.market_runs(&provider()).unwrap();
    assert_eq!(financial[0].run.id, a);
    assert_eq!(financial[1].run.id, c);
    assert_eq!(market[0].run.id, b);
    assert!(
        financial[0].context.sequence < market[0].context.sequence
            && market[0].context.sequence < financial[1].context.sequence
    );
    assert_eq!(
        financial[0].facts[0].retrieval.observation_id,
        financial[1].facts[0].retrieval.observation_id
    );
    assert_ne!(
        financial[0].facts[0].value.retrieved_at,
        financial[1].facts[0].value.retrieved_at
    );
    let identity = financial[0].context.repository_id.clone();
    let last = financial[1].context.sequence;
    drop(repo);
    let mut repo = SqliteRepository::open(&path).unwrap();
    save_facts(&mut repo, vec![]);
    let data = repo.financial_runs(&provider()).unwrap();
    assert_eq!(data.last().unwrap().context.repository_id, identity);
    assert!(data.last().unwrap().context.sequence > last);
    let other = SqliteRepository::open(":memory:").unwrap();
    assert!(other.financial_runs(&provider()).unwrap().is_empty());
}
#[test]
fn form_date_and_operation_filters_preserve_absence() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let mut amendment = fact("2", "2024-03-01", instant("2023-12-31"));
    amendment.form = "10-K/A".into();
    let a = save_facts(
        &mut repo,
        vec![fact("1", "2024-02-01", instant("2023-12-31")), amendment],
    );
    let filings = repo.start_run(&provider(), &scope(), "filings").unwrap();
    repo.save_filings_page(
        filings,
        &Page {
            items: vec![Filing {
                company: scope().company,
                filing_id: "doc".into(),
                form: "10-K".into(),
                filed: "2024-02-01".parse().unwrap(),
                report_date: None,
                accepted_at: None,
                primary_document: None,
                source_url: "https://fixture.test/filing".into(),
                retrieved_at: "2024-02-01T12:00:00Z".parse().unwrap(),
            }],
            next_cursor: None,
        },
    )
    .unwrap();
    repo.finish_run(filings, None).unwrap();
    let data = repo.financial_runs(&provider()).unwrap();
    assert_eq!(data[1].filings[0].retrieval.kind, "filing");
    assert!(!data[1].filings[0].retrieval.fingerprint.is_empty());
    let mut q = metric();
    q.scope.forms = vec!["10-K".into()];
    let s = select_facts(&provider(), &q, &data, None).unwrap();
    assert_eq!(s.manifest.selected_run.unwrap().run_id, a);
    assert_eq!(s.groups[0].value.as_ref().unwrap().as_str(), "1");
    q.scope.forms.clear();
    q.scope.filed_to = "2024-02-20".parse().unwrap();
    assert_eq!(
        select_facts(&provider(), &q, &data, None).unwrap().groups[0]
            .value
            .as_ref()
            .unwrap()
            .as_str(),
        "1"
    );
    q = metric();
    assert_eq!(
        select_facts(&provider(), &q, &data, None).unwrap().groups[0]
            .value
            .as_ref()
            .unwrap()
            .as_str(),
        "2"
    );
    save_facts(&mut repo, vec![]);
    assert!(
        select_facts(
            &provider(),
            &q,
            &repo.financial_runs(&provider()).unwrap(),
            None
        )
        .unwrap()
        .groups
        .is_empty()
    );
    q.concept = "Unmapped".into();
    assert!(
        select_facts(&provider(), &q, &data, Some(a))
            .unwrap()
            .groups
            .is_empty()
    );
}
#[test]
fn normalized_transport_and_forms_do_not_change_manifest() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    save_facts(
        &mut repo,
        vec![fact("1", "2024-02-01", instant("2023-12-31"))],
    );
    let data = repo.financial_runs(&provider()).unwrap();
    let mut q = metric();
    q.scope.forms = vec!["10-K".into(), "10-Q".into()];
    let one = select_facts(&provider(), &q, &data, None).unwrap();
    q.scope.forms = vec!["10-Q".into(), "10-K".into(), "10-K".into()];
    q.scope.cursor = Some("transport".into());
    q.scope.page_size = 1;
    assert_eq!(
        serde_json::to_value(one).unwrap(),
        serde_json::to_value(select_facts(&provider(), &q, &data, None).unwrap()).unwrap()
    );
}
#[test]
fn explicit_source_partial_marker_and_invalid_chronology() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let id = repo.start_market_run(&provider(), &query()).unwrap();
    let mut p = page(vec![bar("1", "2024-01-02")]);
    p.coverage.completeness = Completeness::Partial;
    repo.save_prices_page(id, &p).unwrap();
    repo.finish_market_run(id, None).unwrap();
    let mut data = repo.market_runs(&provider()).unwrap();
    let selected = select_daily(&provider(), &query(), PriceSeries::Close, &data, None).unwrap();
    assert_eq!(
        selected.coverage.unwrap().completeness,
        Completeness::Partial
    );
    assert_eq!(
        selected.manifest.selected_run.unwrap().status,
        RunStatus::Complete
    );
    data.push(data[0].clone());
    assert!(select_daily(&provider(), &query(), PriceSeries::Close, &data, None).is_err());
    data[1].context.sequence += 1;
    data[1].context.repository_id = "different-database".into();
    assert!(select_daily(&provider(), &query(), PriceSeries::Close, &data, None).is_err());
}
#[test]
fn saved_manifest_freezes_source_coverage_across_refresh() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let old = repo.start_market_run(&provider(), &query()).unwrap();
    let mut partial = page(vec![bar("1", "2024-01-02")]);
    partial.coverage.completeness = Completeness::Partial;
    repo.save_prices_page(old, &partial).unwrap();
    repo.finish_market_run(old, None).unwrap();
    let selected = select_daily(
        &provider(),
        &query(),
        PriceSeries::Close,
        &repo.market_runs(&provider()).unwrap(),
        None,
    )
    .unwrap();
    let frozen = serde_json::to_string(&selected.manifest).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&frozen).unwrap()["source_coverage"]["completeness"],
        "partial"
    );
    let new = repo.start_market_run(&provider(), &query()).unwrap();
    let mut complete = page(vec![bar("2", "2024-01-02"), bar("3", "2024-01-03")]);
    complete.coverage.completeness = Completeness::Complete;
    repo.save_prices_page(new, &complete).unwrap();
    repo.finish_market_run(new, None).unwrap();
    let refreshed = select_daily(
        &provider(),
        &query(),
        PriceSeries::Close,
        &repo.market_runs(&provider()).unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&refreshed.manifest).unwrap()["source_coverage"],
        serde_json::to_value(&complete.coverage).unwrap()
    );
    let reopened: SelectionManifest = serde_json::from_str(&frozen).unwrap();
    assert_eq!(
        serde_json::to_value(reopened).unwrap()["source_coverage"],
        serde_json::to_value(&partial.coverage).unwrap()
    );
    assert_eq!(serde_json::to_string(&selected.manifest).unwrap(), frozen);
}
