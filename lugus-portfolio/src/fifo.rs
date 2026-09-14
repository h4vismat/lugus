use crate::*;
pub(crate) fn trade(state: &mut AccountState, e: &Event) -> Result<()> {
    let (buy, instrument_id, quantity, price, gross, fees, overridden) = match &e.kind {
        EventKind::Buy {
            instrument_id,
            quantity,
            price,
            gross,
            fees,
            gross_overridden,
        } => (
            true,
            instrument_id,
            quantity,
            price,
            gross,
            fees,
            *gross_overridden,
        ),
        EventKind::Sell {
            instrument_id,
            quantity,
            price,
            gross,
            fees,
            gross_overridden,
        } => (
            false,
            instrument_id,
            quantity,
            price,
            gross,
            fees,
            *gross_overridden,
        ),
        _ => unreachable!(),
    };
    require(
        quantity.is_positive()
            && price.is_positive()
            && gross.is_positive()
            && gross.is_cents()
            && !fees.is_negative()
            && fees.is_cents(),
        "invalid trade quantity, price, gross or fees",
    )?;
    require(
        overridden || quantity.checked_mul(price)?.round_cents()? == *gross,
        "gross differs from rounded quantity × price; confirm an override",
    )?;
    state.trade_fees = state.trade_fees.checked_add(fees)?;
    if buy {
        let basis = gross.checked_add(fees)?;
        state.cash = state.cash.checked_sub(&basis)?;
        state.lots.push(Lot {
            id: e.id.clone(),
            instrument_id: instrument_id.clone(),
            acquired: e.date,
            tie_order: e.order,
            quantity: quantity.clone(),
            basis,
            simplified: false,
        });
    } else {
        require(fees <= gross, "sale fees exceed proceeds")?;
        let total = state
            .lots
            .iter()
            .filter(|l| l.instrument_id == *instrument_id)
            .try_fold(Decimal::zero(), |v, l| v.checked_add(&l.quantity))?;
        if total < *quantity {
            return Err(PortfolioError::InsufficientShares {
                event_id: e.id.clone(),
            });
        }
        let net = gross.checked_sub(fees)?;
        state.cash = state.cash.checked_add(&net)?;
        state
            .lots
            .sort_by_key(|l| (l.acquired, l.tie_order, l.id.clone()));
        let mut left = quantity.clone();
        let mut proceeds = net.clone();
        for l in &mut state.lots {
            if l.instrument_id != *instrument_id || left.is_zero() {
                continue;
            }
            let consumed = left.clone().min(l.quantity.clone());
            let basis = if consumed == l.quantity {
                l.basis.clone()
            } else {
                l.basis.allocated(&consumed, &l.quantity)?
            };
            let allocated = if consumed == left {
                proceeds.clone()
            } else {
                net.allocated(&consumed, quantity)?
            };
            let realized = allocated.checked_sub(&basis)?;
            state.realized = state.realized.checked_add(&realized)?;
            state.matches.push(SaleMatch {
                sale_id: e.id.clone(),
                lot_id: l.id.clone(),
                quantity: consumed.clone(),
                basis: basis.clone(),
                net_proceeds: allocated.clone(),
                realized,
                simplified: l.simplified,
            });
            l.quantity = l.quantity.checked_sub(&consumed)?;
            l.basis = l.basis.checked_sub(&basis)?;
            left = left.checked_sub(&consumed)?;
            proceeds = proceeds.checked_sub(&allocated)?;
        }
        state.lots.retain(|l| !l.quantity.is_zero());
    }
    Ok(())
}
