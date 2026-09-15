use lugus_app::portfolio::*;
use serde_json::json;
#[test]
fn metrics_preserve_exact_cash_and_incomplete_valuations() {
    let mut view:PortfolioView=serde_json::from_value(json!({"id":"p","name":"P","revision":"1","as_of":"2026-01-01","accounts":[],"instruments":[],"valuation":{"cash":"9007199254740993.01","holdings":[],"priced_subtotal":"9007199254740993.01","total_value":"9007199254740993.01","unrealized":"0","complete":true,"cash_allocation_percent":"100"},"realized":"0","dividends":"0","standalone_fees":"0","trade_fees":"0","deposits":"0","withdrawals":"0","price_status":[]})).unwrap();
    let metrics = dashboard_metrics(&view).unwrap();
    assert_eq!(metrics.invested_market_value.unwrap().to_string(), "0");
    assert_eq!(
        metrics.by_asset_type[0].value.to_string(),
        "9007199254740993.01"
    );
    assert_eq!(metrics.by_asset_type[0].percent.to_string(), "100");
    assert_eq!(metrics.largest_weight, None);
    view.valuation.complete = false;
    view.valuation.total_value = None;
    let metrics = dashboard_metrics(&view).unwrap();
    assert_eq!(metrics.invested_market_value, None);
    assert!(metrics.by_asset_type.is_empty());
}

#[test]
fn metrics_group_asset_types_and_keep_zero_basis_return_unavailable() {
    let view: PortfolioView = serde_json::from_value(json!({
        "id":"p", "name":"P", "revision":"1", "as_of":"2026-01-01",
        "accounts":[], "instruments":[
            {"id":"stock", "name":"Stock", "symbol":"S", "asset_kind":"stock", "currency":"USD", "binding":null},
            {"id":"fund", "name":"Fund", "symbol":"F", "asset_kind":"etf", "currency":"USD", "binding":null}
        ],
        "valuation":{
            "cash":"100", "priced_subtotal":"1000", "total_value":"1000", "unrealized":"600",
            "complete":true, "cash_allocation_percent":"10", "holdings":[
                {"instrument_id":"stock", "quantity":"6", "basis":"300", "market_value":"600", "unrealized":"300", "price":null, "unpriced_reason":null, "allocation_percent":"60", "simplified":false},
                {"instrument_id":"fund", "quantity":"3", "basis":"0", "market_value":"300", "unrealized":"300", "price":null, "unpriced_reason":null, "allocation_percent":"30", "simplified":false}
            ]
        },
        "realized":"0", "dividends":"0", "standalone_fees":"0", "trade_fees":"0", "deposits":"0", "withdrawals":"0", "price_status":[]
    })).unwrap();
    let metrics = dashboard_metrics(&view).unwrap();
    assert_eq!(metrics.invested_market_value.unwrap().to_string(), "900");
    assert_eq!(metrics.largest_instrument_id.as_deref(), Some("stock"));
    assert_eq!(metrics.largest_weight.unwrap().to_string(), "60");
    assert_eq!(metrics.top_two_weight.unwrap().to_string(), "90");
    assert_eq!(
        metrics
            .by_asset_type
            .iter()
            .map(|a| (a.key.as_str(), a.percent.to_string()))
            .collect::<Vec<_>>(),
        vec![
            ("stocks", "60".into()),
            ("etfs", "30".into()),
            ("cash", "10".into())
        ]
    );
    assert_eq!(
        metrics.holdings[0]
            .unrealized_percent
            .as_ref()
            .unwrap()
            .to_string(),
        "100"
    );
    assert_eq!(metrics.holdings[1].unrealized_percent, None);
}
