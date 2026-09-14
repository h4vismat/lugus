use crate::*;
use std::collections::BTreeSet;
fn id(s: &str) -> Result<()> {
    require(
        !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control),
        "invalid identifier",
    )
}
fn cash(v: &Decimal, positive: bool) -> Result<()> {
    require(
        v.is_cents() && !v.is_negative() && (!positive || v.is_positive()),
        "invalid USD cash amount",
    )
}
/// Replay the entire effective ledger through an explicit accounting date.
pub fn replay(l: &Ledger, as_of: Day) -> Result<AccountState> {
    id(&l.account_id)?;
    require(l.start <= as_of, "account starts after accounting date")?;
    require(l.events.len() <= 100_000, "account event limit exceeded")?;
    let mut s = AccountState {
        account_id: l.account_id.clone(),
        start: l.start,
        as_of,
        cash: Decimal::zero(),
        realized: Decimal::zero(),
        dividends: Decimal::zero(),
        standalone_fees: Decimal::zero(),
        trade_fees: Decimal::zero(),
        deposits: Decimal::zero(),
        withdrawals: Decimal::zero(),
        lots: vec![],
        matches: vec![],
    };
    let mut ids = BTreeSet::new();
    let mut ties = BTreeSet::new();
    if let Opening::Existing {
        cash: balance,
        lots,
    } = &l.opening
    {
        cash(balance, false)?;
        s.cash = balance.clone();
        require(lots.len() <= 1000, "opening lot limit exceeded")?;
        for o in lots {
            id(&o.id)?;
            id(&o.instrument_id)?;
            require(
                ids.insert(o.id.clone())
                    && ties.insert((o.instrument_id.clone(), o.acquired, o.tie_order)),
                "duplicate opening lot ID or order",
            )?;
            require(
                o.acquired <= l.start && o.quantity.is_positive() && !o.basis.is_negative(),
                "invalid opening lot",
            )?;
            require(
                !o.date_assumed || (o.simplified && o.acquired == l.start),
                "assumed acquisition date must use the starting date",
            )?;
            s.lots.push(Lot {
                id: o.id.clone(),
                instrument_id: o.instrument_id.clone(),
                acquired: o.acquired,
                tie_order: o.tie_order,
                quantity: o.quantity.clone(),
                basis: o.basis.clone(),
                simplified: o.simplified,
            });
        }
    }
    let mut events = l.events.iter().collect::<Vec<_>>();
    events.sort_by_key(|e| (e.date, e.order));
    let mut orders = BTreeSet::new();
    let mut actions = BTreeSet::new();
    for e in events {
        id(&e.id)?;
        require(e.date >= l.start, "transaction precedes starting date")?;
        if !ids.insert(e.id.clone()) || !orders.insert((e.date, e.order)) {
            return Err(PortfolioError::InvalidOrder);
        }
        if let Some(instrument) = e.kind.instrument_id() {
            id(instrument)?;
        }
        if e.date > as_of {
            continue;
        }
        match &e.kind {
            EventKind::Buy { instrument_id, .. } => {
                require(
                    ties.insert((instrument_id.clone(), e.date, e.order)),
                    "purchase and opening lot have the same FIFO order",
                )?;
                crate::fifo::trade(&mut s, e)?;
            }
            EventKind::Sell { .. } => crate::fifo::trade(&mut s, e)?,
            EventKind::Deposit { amount } => {
                cash(amount, true)?;
                s.cash = s.cash.checked_add(amount)?;
                s.deposits = s.deposits.checked_add(amount)?;
            }
            EventKind::Withdrawal { amount } => {
                cash(amount, true)?;
                s.cash = s.cash.checked_sub(amount)?;
                s.withdrawals = s.withdrawals.checked_add(amount)?;
            }
            EventKind::Dividend { amount, .. } => {
                cash(amount, true)?;
                s.cash = s.cash.checked_add(amount)?;
                s.dividends = s.dividends.checked_add(amount)?;
            }
            EventKind::Fee { amount } => {
                cash(amount, true)?;
                s.cash = s.cash.checked_sub(amount)?;
                s.standalone_fees = s.standalone_fees.checked_add(amount)?;
            }
            EventKind::Split {
                instrument_id,
                numerator,
                denominator,
                action_id,
            } => {
                id(action_id)?;
                require(
                    actions.insert(action_id.clone()),
                    "split action already applied",
                )?;
                require(*numerator > 0 && *denominator > 0, "invalid split ratio")?;
                for lot in s
                    .lots
                    .iter_mut()
                    .filter(|lot| lot.instrument_id == *instrument_id)
                {
                    lot.quantity = lot.quantity.split_exact(*numerator, *denominator)?;
                }
            }
        }
        if s.cash.is_negative() {
            return Err(PortfolioError::InsufficientCash {
                event_id: e.id.clone(),
            });
        }
    }
    Ok(s)
}
