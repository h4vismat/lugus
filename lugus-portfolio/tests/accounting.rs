use lugus_portfolio::*;
use serde_json::{Value, json};
fn d(s: &str) -> Decimal {
    Decimal::parse(s).unwrap()
}
fn day() -> Day {
    "2026-01-02".parse().unwrap()
}
fn event(id: &str, order: u64, kind: Value) -> Value {
    json!({"id":id,"date":"2026-01-01","order":order,"kind":kind})
}
fn ledger(events: Vec<Value>) -> Ledger {
    serde_json::from_value(json!({"account_id":"a","start":"2026-01-01","opening":{"kind":"full_history"},"events":events})).unwrap()
}
fn deposit(amount: &str) -> Value {
    event("deposit", 0, json!({"kind":"deposit","amount":amount}))
}
fn trade(
    kind: &str,
    id: &str,
    order: u64,
    qty: &str,
    price: &str,
    gross: &str,
    fees: &str,
) -> Value {
    event(
        id,
        order,
        json!({"kind":kind,"instrument_id":"stock","quantity":qty,"price":price,"gross":gross,"fees":fees,"gross_overridden":false}),
    )
}
fn example() -> Ledger {
    ledger(vec![
        deposit("200"),
        trade("buy", "b1", 1, "10", "10", "100", "1"),
        trade("buy", "b2", 2, "5", "12", "60", "0"),
        trade("sell", "s1", 3, "12", "15", "180", "2"),
    ])
}
#[test]
fn decimal_exact_and_half_even() {
    assert_eq!(d("0.1").checked_add(&d("0.2")).unwrap(), d("0.3"));
    for (n, w) in [
        ("1.005", "1"),
        ("1.015", "1.02"),
        ("-1.005", "-1"),
        ("-1.015", "-1.02"),
    ] {
        assert_eq!(d(n).round_cents().unwrap(), d(w));
    }
    assert!(Decimal::parse("0.0000000000000000001").is_err());
    assert!(Decimal::parse("100000000000000000000").is_err());
    assert!(Decimal::parse("1e5").is_err());
    assert!(serde_json::from_str::<Decimal>("0.1").is_err());
    assert_eq!(serde_json::to_string(&d("-0.000")).unwrap(), "\"0\"");
    assert!(d("1").allocated(&d("1"), &d("0")).is_err());
}
#[test]
fn fifo_example() {
    let s = replay(&example(), day()).unwrap();
    assert_eq!(s.cash, d("217"));
    assert_eq!(s.realized, d("53"));
    assert_eq!(s.lots.len(), 1);
    assert_eq!(s.lots[0].quantity, d("3"));
    assert_eq!(s.lots[0].basis, d("36"));
    assert_eq!(s.matches.len(), 2);
}
#[test]
fn intermediate_negative_cash_is_rejected() {
    let l = ledger(vec![
        event("w", 0, json!({"kind":"withdrawal","amount":"1"})),
        event("d", 1, json!({"kind":"deposit","amount":"10"})),
    ]);
    assert!(
        matches!(replay(&l,day()),Err(PortfolioError::InsufficientCash{event_id}) if event_id=="w")
    );
}
#[test]
fn partial_disposals_conserve_basis() {
    let mut l = ledger(vec![
        deposit("2"),
        trade("buy", "b", 1, "3", "0.333333333333333333", "1", "0"),
    ]);
    for n in 2..5 {
        l.events.push(
            serde_json::from_value(trade("sell", &format!("s{n}"), n, "1", "1", "1", "0")).unwrap(),
        );
    }
    let s = replay(&l, day()).unwrap();
    assert!(s.lots.is_empty());
    assert_eq!(s.realized, d("2"));
    let basis = s
        .matches
        .iter()
        .try_fold(d("0"), |v, m| v.checked_add(&m.basis))
        .unwrap();
    assert_eq!(basis, d("1"));
}
#[test]
fn opening_lots_and_split_preserve_basis() {
    let mut l = example();
    l.events=vec![serde_json::from_value(event("split",0,json!({"kind":"split","instrument_id":"stock","numerator":2,"denominator":1,"action_id":"split-1"}))).unwrap()];
    l.opening = Opening::Existing {
        cash: d("39"),
        lots: vec![OpeningLot {
            id: "b".into(),
            instrument_id: "stock".into(),
            acquired: "2025-01-01".parse().unwrap(),
            tie_order: 1,
            quantity: d("10"),
            basis: d("101"),
            simplified: true,
            date_assumed: false,
        }],
    };
    let s = replay(&l, day()).unwrap();
    assert_eq!(s.lots[0].quantity, d("20"));
    assert_eq!(s.lots[0].basis, d("101"));
    assert_eq!(s.deposits, d("0"));
    assert_eq!(s.cash, d("39"));
}
#[test]
fn rejects_oversell_duplicate_order_and_inexact_split() {
    let mut l = example();
    l.events
        .push(serde_json::from_value(trade("sell", "s2", 4, "4", "15", "60", "0")).unwrap());
    assert!(replay(&l, day()).is_err());
    let mut l = example();
    l.events[1].order = 0;
    assert!(replay(&l, day()).is_err());
    let mut l = ledger(vec![
        deposit("10"),
        trade("buy", "b", 1, "1", "1", "1", "0"),
    ]);
    l.events.push(serde_json::from_value(event("split",2,json!({"kind":"split","instrument_id":"stock","numerator":1,"denominator":3,"action_id":"s"}))).unwrap());
    assert!(replay(&l, day()).is_err());
}
#[test]
fn valuation_distinguishes_unpriced_and_cash_only() {
    let state = replay(&example(), day()).unwrap();
    let v = value_accounts(std::slice::from_ref(&state), &[], day()).unwrap();
    assert!(!v.complete);
    assert_eq!(v.priced_subtotal, d("217"));
    assert!(v.total_value.is_none());
    let p = PriceInput {
        instrument_id: "stock".into(),
        close: d("15"),
        currency: "USD".into(),
        date: day(),
        basis: PriceBasis::Compatible {
            share_basis_date: day(),
        },
        observation_id: "price1".into(),
    };
    let v = value_accounts(&[state], &[p], day()).unwrap();
    assert_eq!(v.total_value, Some(d("262")));
    assert_eq!(v.unrealized, Some(d("9")));
    let cash = replay(&ledger(vec![deposit("10")]), day()).unwrap();
    assert!(value_accounts(&[cash], &[], day()).unwrap().complete);
}
#[test]
fn opening_and_new_purchase_cannot_have_ambiguous_fifo_order() {
    let mut l = ledger(vec![trade("buy", "new", 0, "1", "1", "1", "0")]);
    l.opening = Opening::Existing {
        cash: d("1"),
        lots: vec![OpeningLot {
            id: "old".into(),
            instrument_id: "stock".into(),
            acquired: l.start,
            tie_order: 0,
            quantity: d("1"),
            basis: d("1"),
            simplified: false,
            date_assumed: false,
        }],
    };
    assert!(replay(&l, day()).is_err());
}
