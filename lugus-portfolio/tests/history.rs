use lugus_portfolio::*;
use serde_json::json;
#[test]
fn incremental_replay_matches_accounting_and_rejects_backwards_dates() {
    let ledger: Ledger = serde_json::from_value(
        json!({"account_id":"a","start":"2026-01-01","opening":{"kind":"full_history"},"events":[
            {"id":"d","date":"2026-01-01","order":0,"kind":{"kind":"deposit","amount":"100"}},
            {"id":"w","date":"2026-01-03","order":0,"kind":{"kind":"withdrawal","amount":"25"}}
        ]}),
    )
    .unwrap();
    let mut cursor = ReplayCursor::new(&ledger).unwrap();
    for text in ["2026-01-01", "2026-01-02", "2026-01-03"] {
        let day = text.parse().unwrap();
        assert_eq!(
            cursor.advance_to(day).unwrap(),
            &replay(&ledger, day).unwrap()
        );
    }
    assert!(cursor.advance_to("2026-01-02".parse().unwrap()).is_err());
}
#[test]
fn cancellation_is_checked_before_advancing() {
    let ledger:Ledger=serde_json::from_value(json!({"account_id":"a","start":"2026-01-01","opening":{"kind":"full_history"},"events":[]})).unwrap();
    let mut cursor = ReplayCursor::new(&ledger).unwrap();
    assert_eq!(
        cursor.advance_to_cancellable(ledger.start, &|| true),
        Err(PortfolioError::Cancelled)
    );
}
fn d(s: &str) -> Decimal {
    Decimal::parse(s).unwrap()
}
#[test]
fn existing_account_enters_at_market_value_and_missing_sessions_are_gaps() {
    let ledgers:Vec<Ledger>=serde_json::from_value(json!([{"account_id":"a","start":"2026-01-02","opening":{"kind":"existing","cash":"10","lots":[{"id":"l","instrument_id":"x","acquired":"2020-01-01","tie_order":0,"quantity":"2","basis":"5","simplified":false,"date_assumed":false}]},"events":[]}])).unwrap();
    let closes:Vec<HistoricalClose>=serde_json::from_value(json!([
        {"instrument_id":"x","date":"2026-01-01","session_close":"2026-01-01T21:00:00Z","close":"20","split":null,"observation_id":"1","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-02","session_close":"2026-01-02T21:00:00Z","close":"21","split":null,"observation_id":"2","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-03","session_close":null,"close":null,"split":null,"observation_id":"3","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-04","session_close":"2026-01-04T21:00:00Z","close":null,"split":null,"observation_id":"4","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-05","session_close":"2026-01-05T21:00:00Z","close":"22","split":null,"observation_id":"5","unsupported_action":null}
    ])).unwrap();
    let days = historical_values(&ValuationHistoryInput {
        ledgers: &ledgers,
        closes: &closes,
        benchmark: &[],
        baseline: "2026-01-01".parse().unwrap(),
        end: "2026-01-05".parse().unwrap(),
    })
    .unwrap();
    assert_eq!(days[0].value, Some(d("0")));
    assert_eq!(days[1].opening_contribution, Some(d("50")));
    assert_eq!(days[1].value, Some(d("52")));
    assert_eq!(days[2].value, Some(d("52")));
    assert_eq!(days[3].value, None);
    assert_eq!(days[4].value, Some(d("54")));
}
#[test]
fn split_evidence_is_reconciled_per_account_and_mismatch_persists() {
    for (n, den, quantity, before, after) in [(2, 1, "10", "20", "10"), (1, 10, "10", "20", "200")]
    {
        let ledger:Ledger=serde_json::from_value(json!({"account_id":"a","start":"2026-01-01","opening":{"kind":"existing","cash":"0","lots":[{"id":"l","instrument_id":"x","acquired":"2020-01-01","tie_order":0,"quantity":quantity,"basis":"5","simplified":false,"date_assumed":false}]},"events":[{"id":"split","date":"2026-01-02","order":0,"kind":{"kind":"split","instrument_id":"x","numerator":n,"denominator":den,"action_id":"action"}}]})).unwrap();
        let closes:Vec<HistoricalClose>=serde_json::from_value(json!([
            {"instrument_id":"x","date":"2025-12-31","session_close":"2025-12-31T21:00:00Z","close":before,"split":null,"observation_id":"0","unsupported_action":null},
            {"instrument_id":"x","date":"2026-01-01","session_close":"2026-01-01T21:00:00Z","close":before,"split":null,"observation_id":"1","unsupported_action":null},
            {"instrument_id":"x","date":"2026-01-02","session_close":"2026-01-02T21:00:00Z","close":after,"split":[n,den],"observation_id":"2","unsupported_action":null},
            {"instrument_id":"x","date":"2026-01-03","session_close":"2026-01-03T21:00:00Z","close":after,"split":null,"observation_id":"3","unsupported_action":null}
        ])).unwrap();
        let mut ledgers = vec![ledger];
        let run = |ledgers: &[Ledger]| {
            historical_values(&ValuationHistoryInput {
                ledgers,
                closes: &closes,
                benchmark: &[],
                baseline: "2026-01-01".parse().unwrap(),
                end: "2026-01-03".parse().unwrap(),
            })
            .unwrap()
        };
        let values = run(&ledgers);
        assert!(values.iter().all(|v| v.value == Some(d("200"))));
        assert_eq!(
            replay(&ledgers[0], "2026-01-03".parse().unwrap())
                .unwrap()
                .lots[0]
                .basis,
            d("5")
        );
        ledgers[0].events.clear();
        let values = run(&ledgers);
        assert_eq!(values[1].value, None);
        assert_eq!(values[2].value, None);
        assert!(
            values[2]
                .issues
                .iter()
                .any(|i| i.code == HistoryIssueCode::SplitMismatch)
        );
    }
}
#[test]
fn split_chain_rounds_only_once_after_exact_ratio_product() {
    assert_eq!(
        d("3").adjusted_by_splits(&[(1, 3), (3, 1)]).unwrap(),
        d("3")
    );
    assert_eq!(
        d("499.23").adjusted_by_splits(&[(1, 4)]).unwrap(),
        d("124.8075")
    );
}
#[test]
fn dividend_cash_offsets_price_drop_and_fees_reduce_returns_once() {
    let ledgers:Vec<Ledger>=serde_json::from_value(json!([{"account_id":"a","start":"2026-01-05","opening":{"kind":"full_history"},"events":[
        {"id":"deposit","date":"2026-01-05","order":0,"kind":{"kind":"deposit","amount":"100"}},
        {"id":"buy","date":"2026-01-05","order":1,"kind":{"kind":"buy","instrument_id":"x","quantity":"1","price":"100","gross":"100","fees":"0","gross_overridden":false}},
        {"id":"dividend","date":"2026-01-06","order":0,"kind":{"kind":"dividend","instrument_id":"x","amount":"5"}},
        {"id":"fee","date":"2026-01-07","order":0,"kind":{"kind":"fee","amount":"1"}}
    ]}])).unwrap();
    let closes:Vec<HistoricalClose>=serde_json::from_value(json!([
        {"instrument_id":"x","date":"2026-01-05","session_close":"2026-01-05T21:00:00Z","close":"100","split":null,"observation_id":"1","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-06","session_close":"2026-01-06T21:00:00Z","close":"95","split":null,"observation_id":"2","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-07","session_close":"2026-01-07T21:00:00Z","close":"95","split":null,"observation_id":"3","unsupported_action":null}
    ])).unwrap();
    let days = historical_values(&ValuationHistoryInput {
        ledgers: &ledgers,
        closes: &closes,
        benchmark: &[],
        baseline: "2026-01-05".parse().unwrap(),
        end: "2026-01-07".parse().unwrap(),
    })
    .unwrap();
    assert_eq!(days[1].value, Some(d("100")));
    assert_eq!(days[1].deposits, Some(d("0")));
    let returns = calculate_performance(&days).unwrap();
    assert_eq!(returns.points[1].portfolio_return_percent, Some(d("0")));
    assert_eq!(returns.summary.portfolio_return_percent, Some(d("-1")));
}
#[test]
fn later_source_split_does_not_invalidate_a_fully_sold_position() {
    let ledgers:Vec<Ledger>=serde_json::from_value(json!([{"account_id":"a","start":"2026-01-05","opening":{"kind":"existing","cash":"0","lots":[{"id":"l","instrument_id":"x","acquired":"2020-01-01","tie_order":0,"quantity":"1","basis":"50","simplified":false,"date_assumed":false}]},"events":[{"id":"sell","date":"2026-01-06","order":0,"kind":{"kind":"sell","instrument_id":"x","quantity":"1","price":"100","gross":"100","fees":"0","gross_overridden":false}}]}])).unwrap();
    let closes:Vec<HistoricalClose>=serde_json::from_value(json!([
        {"instrument_id":"x","date":"2026-01-05","session_close":"2026-01-05T21:00:00Z","close":"100","split":null,"observation_id":"1","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-06","session_close":"2026-01-06T21:00:00Z","close":"100","split":null,"observation_id":"2","unsupported_action":null},
        {"instrument_id":"x","date":"2026-01-07","session_close":"2026-01-07T21:00:00Z","close":"50","split":[2,1],"observation_id":"3","unsupported_action":null}
    ])).unwrap();
    let days = historical_values(&ValuationHistoryInput {
        ledgers: &ledgers,
        closes: &closes,
        benchmark: &[],
        baseline: "2026-01-05".parse().unwrap(),
        end: "2026-01-07".parse().unwrap(),
    })
    .unwrap();
    assert!(
        days.iter()
            .all(|d| d.value == Some(Decimal::parse("100").unwrap()))
    );
    assert!(
        !days[2]
            .issues
            .iter()
            .any(|i| i.code == HistoryIssueCode::SplitMismatch)
    );
}
