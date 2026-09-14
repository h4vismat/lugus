mod support;
use lugus_app::{portfolio::*, *};
use serde_json::json;
#[tokio::test]
async fn refresh_saves_close_valuation_and_survives_offline_reopen() {
    let h = support::Harness::with_plugin(
        &[("p", "portfolio_current")],
        HostBounds::default(),
        Limits::default(),
        "yfinance",
        "0.2.0",
    )
    .await;
    let created = h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"create","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"P"}})).unwrap()).await.unwrap();
    let p = created.portfolio_id;
    let instrument = h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"instrument","portfolio_id":p,"expected_revision":"1","mutation":{"kind":"create_instrument","name":"Test","symbol":"TEST","asset_kind":"stock"}})).unwrap()).await.unwrap().instrument_ids[0].clone();
    h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"account","portfolio_id":p,"expected_revision":"2","mutation":{"kind":"create_account","name":"Broker","start":"2026-01-01","opening":{"kind":"existing","cash":"100","lots":[{"id":"lot","instrument_id":instrument,"acquired":"2025-01-01","tie_order":1,"quantity":"1","basis":"100","simplified":false,"date_assumed":false}]},"events":[]}})).unwrap()).await.unwrap();
    h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"binding","portfolio_id":p,"expected_revision":"3","mutation":{"kind":"bind_instrument","instrument_id":instrument,"instance_id":"p","native_id":{"namespace":"yahoo:symbol","value":"TEST"}}})).unwrap()).await.unwrap();
    let mut result = h
        .app
        .refresh_portfolio(RefreshRequest {
            request_id: "refresh".into(),
            portfolio_id: p.clone(),
            expected_revision: 4,
        })
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while result.status == "running" {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            result = h
                .app
                .portfolio_refresh_status(result.id.clone())
                .await
                .unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(result.status, "complete", "{result:?}");
    assert_eq!(
        result.receipts[0].price.as_ref().unwrap().close.to_string(),
        "101"
    );
    let view = h.app.portfolio_overview(p.clone(), None).await.unwrap();
    assert_eq!(
        view.valuation.total_value.as_ref().unwrap().to_string(),
        "201"
    );
    assert_eq!(view.valuation.unrealized.as_ref().unwrap().to_string(), "1");
    h.app.shutdown().await.unwrap();
    let store = SqliteApplicationStore::open(
        &h.application,
        Box::new(lugus_financial::storage::SqliteRepository::open(&h.financial).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    assert_eq!(store.portfolio_overview(&p, None).unwrap(), view);
    let mut store = store;
    let (orphan, _) = store
        .portfolio_refresh_begin(&RefreshRequest {
            request_id: "orphan".into(),
            portfolio_id: p,
            expected_revision: 4,
        })
        .unwrap();
    assert_eq!(
        h.app
            .portfolio_refresh_status(orphan.id)
            .await
            .unwrap()
            .status,
        "interrupted_or_external"
    );
}
#[tokio::test]
async fn cash_only_refresh_is_durable_and_idempotent() {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let r=h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"p","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"P"}})).unwrap()).await.unwrap();
    let request = RefreshRequest {
        request_id: "refresh".into(),
        portfolio_id: r.portfolio_id.clone(),
        expected_revision: 1,
    };
    let mut result = h.app.refresh_portfolio(request.clone()).await.unwrap();
    while result.status == "running" {
        tokio::task::yield_now().await;
        result = h
            .app
            .portfolio_refresh_status(result.id.clone())
            .await
            .unwrap();
    }
    assert_eq!(result.status, "complete");
    assert_eq!(
        serde_json::to_value(h.app.refresh_portfolio(request).await.unwrap()).unwrap(),
        serde_json::to_value(result).unwrap()
    );
    assert_eq!(
        h.app
            .portfolio_overview(r.portfolio_id, None)
            .await
            .unwrap()
            .revision,
        1
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn blocked_refresh_can_be_cancelled_and_shutdown_waits_for_cleanup() {
    let h = support::Harness::new(
        &[("source", "apple_price_blocked")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let p=h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"p","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"P"}})).unwrap()).await.unwrap().portfolio_id;
    let instrument=h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"i","portfolio_id":p,"expected_revision":"1","mutation":{"kind":"create_instrument","name":"Stock","symbol":"TEST","asset_kind":"stock"}})).unwrap()).await.unwrap().instrument_ids[0].clone();
    h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"bind","portfolio_id":p,"expected_revision":"2","mutation":{"kind":"bind_instrument","instrument_id":instrument,"instance_id":"source","native_id":{"namespace":"yahoo:symbol","value":"TEST"}}})).unwrap()).await.unwrap();
    let result = h
        .app
        .refresh_portfolio(RefreshRequest {
            request_id: "refresh".into(),
            portfolio_id: p,
            expected_revision: 3,
        })
        .await
        .unwrap();
    assert_eq!(result.status, "running");
    h.app.cancel_portfolio_refresh(result.id.clone()).unwrap();
    h.app.shutdown().await.unwrap();
    let terminal = h.app.portfolio_refresh_status(result.id).await.unwrap();
    assert_eq!(terminal.status, "cancelled");
}
#[test]
fn yfinance_adapter_uses_split_only_close_and_rejects_incompatible_dates() {
    use lugus_financial::{domain::ProviderIdentity, market_data::PriceBar};
    let as_of = chrono::NaiveDate::from_ymd_opt(2026, 9, 12).unwrap();
    let provider = ProviderIdentity {
        instance_id: "p".into(),
        plugin_id: "yfinance".into(),
        plugin_version: "0.2.0".into(),
    };
    let binding = PortfolioBinding {
        instance_id: "p".into(),
        native_id: lugus_financial::market_data::InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: "TEST".into(),
        },
    };
    let bar:PriceBar=serde_json::from_value(json!({"instrument":{"namespace":"yahoo:symbol","value":"TEST"},"date":"2026-09-11","open":"15","high":"16","low":"14","close":"15","adjusted_close":"13","volume":1,"currency":"USD","exchange_timezone":"America/New_York","price_basis":"source_reported","precision":"binary_float_source","source_url":"https://finance.yahoo.com/quote/TEST/history/","retrieved_at":"2026-09-12T10:00:00Z"})).unwrap();
    let p = yfinance_price_input("i", &provider, &binding, &bar, 10, None, as_of).unwrap();
    assert_eq!(p.close.to_string(), "15");
    assert!(yfinance_price_input("i", &provider, &binding, &bar, 10, Some(as_of), as_of).is_none());
    let mut unknown = provider.clone();
    unknown.plugin_id = "other".into();
    assert!(yfinance_price_input("i", &unknown, &binding, &bar, 10, None, as_of).is_none());
    let mut stale = bar.clone();
    stale.date = as_of - chrono::Duration::days(15);
    assert!(yfinance_price_input("i", &provider, &binding, &stale, 10, None, as_of).is_none());
}
