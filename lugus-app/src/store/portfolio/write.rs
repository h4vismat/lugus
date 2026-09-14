use super::*;
fn edits(account: &mut Account, changes: &[EventEdit]) -> Result<()> {
    if changes.len() > 1000 {
        return Err(invalid("edit batch exceeds 1000 events"));
    }
    let mut changed = std::collections::BTreeSet::new();
    for edit in changes {
        let id = match edit {
            EventEdit::Append { event } | EventEdit::Replace { event } => &event.id,
            EventEdit::Void { id } => id,
        };
        if !changed.insert(id) {
            return Err(invalid("event edited more than once in one batch"));
        }
        let found = account.ledger.events.iter().position(|e| e.id == *id);
        let edits_split = match edit {
            EventEdit::Append { event } | EventEdit::Replace { event } => {
                matches!(event.kind, EventKind::Split { .. })
            }
            EventEdit::Void { .. } => false,
        };
        if edits_split
            || found.is_some_and(|pos| {
                matches!(account.ledger.events[pos].kind, EventKind::Split { .. })
            })
        {
            return Err(invalid(
                "use the portfolio split action to change linked split events",
            ));
        }
        match edit {
            EventEdit::Append { event } => {
                if found.is_some() {
                    return Err(conflict("event ID already exists"));
                }
                account.ledger.events.push(event.clone());
            }
            EventEdit::Replace { event } => {
                let pos = found.ok_or_else(|| invalid("event not found"))?;
                account.ledger.events[pos] = event.clone();
            }
            EventEdit::Void { .. } => {
                let pos = found.ok_or_else(|| invalid("event not found"))?;
                account.ledger.events.remove(pos);
            }
        }
    }
    Ok(())
}
fn account<'a>(d: &'a mut PortfolioDocument, id: &str) -> Result<&'a mut Account> {
    d.accounts
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| scoped("account not found in portfolio"))
}
fn instrument<'a>(d: &'a mut PortfolioDocument, id: &str) -> Result<&'a mut Instrument> {
    d.instruments
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| scoped("instrument not found in portfolio"))
}
pub(super) fn propose(
    s: &SqliteApplicationStore,
    r: &PortfolioCommand,
) -> Result<(PortfolioDocument, PortfolioReceipt)> {
    crate::portfolio::id(&r.request_id)?;
    bound(r, 1024 * 1024)?;
    let create = matches!(r.mutation, PortfolioMutation::CreatePortfolio { .. });
    let mut d = if create {
        if r.portfolio_id.is_some() || r.expected_revision != 0 {
            return Err(conflict(
                "portfolio creation requires revision zero and no ID",
            ));
        }
        PortfolioDocument {
            header: PortfolioHeader {
                id: s.ids.next_id(),
                name: String::new(),
                currency: "USD".into(),
                revision: 0,
            },
            accounts: vec![],
            instruments: vec![],
        }
    } else {
        read(
            &s.connection,
            r.portfolio_id
                .as_deref()
                .ok_or_else(|| invalid("portfolio ID required"))?,
        )?
    };
    if d.header.revision != r.expected_revision {
        return Err(conflict("portfolio changed; reload and preview again"));
    }
    let original = d.clone();
    let revision = d
        .header
        .revision
        .checked_add(1)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| invalid("revision limit exceeded"))?;
    let mut receipt = PortfolioReceipt {
        request_id: r.request_id.clone(),
        portfolio_id: d.header.id.clone(),
        revision,
        account_ids: vec![],
        instrument_ids: vec![],
    };
    match &r.mutation {
        PortfolioMutation::CreatePortfolio { name: n }
        | PortfolioMutation::RenamePortfolio { name: n } => {
            name(n)?;
            d.header.name = n.trim().into();
        }
        PortfolioMutation::CreateInstrument {
            name: n,
            symbol,
            asset_kind,
        } => {
            name(n)?;
            name(symbol)?;
            if d.instruments.len() >= 1000 {
                return Err(invalid("instrument limit exceeded"));
            }
            let id = s.ids.next_id();
            receipt.instrument_ids.push(id.clone());
            d.instruments.push(Instrument {
                id,
                name: n.clone(),
                symbol: symbol.clone(),
                asset_kind: *asset_kind,
                currency: "USD".into(),
                binding: None,
            });
        }
        PortfolioMutation::CreateAccount {
            name: n,
            start,
            opening,
            events,
        } => {
            name(n)?;
            if d.accounts.len() >= 100 || events.len() > 1000 {
                return Err(invalid("account or initialization batch limit exceeded"));
            }
            let id = s.ids.next_id();
            receipt.account_ids.push(id.clone());
            d.accounts.push(Account {
                id: id.clone(),
                name: n.clone(),
                revision,
                ledger: Ledger {
                    account_id: id,
                    start: *start,
                    opening: opening.clone(),
                    events: events.clone(),
                },
            });
        }
        PortfolioMutation::RenameAccount {
            account_id,
            name: n,
        } => {
            name(n)?;
            let a = account(&mut d, account_id)?;
            a.name = n.clone();
            a.revision = revision;
            receipt.account_ids.push(account_id.clone());
        }
        PortfolioMutation::EditEvents {
            account_id,
            edits: changes,
        } => {
            let a = account(&mut d, account_id)?;
            edits(a, changes)?;
            a.revision = revision;
            receipt.account_ids.push(account_id.clone());
        }
        PortfolioMutation::ChangeSetup {
            account_id,
            start,
            opening,
            edits: changes,
        } => {
            let a = account(&mut d, account_id)?;
            a.ledger.start = *start;
            a.ledger.opening = opening.clone();
            edits(a, changes)?;
            a.revision = revision;
            receipt.account_ids.push(account_id.clone());
        }
        PortfolioMutation::ApplySplit {
            instrument_id,
            date,
            order,
            numerator,
            denominator,
        } => {
            instrument(&mut d, instrument_id)?;
            if *numerator == 0 || *denominator == 0 || date > &s.clock.now().date_naive() {
                return Err(invalid("invalid split ratio or date"));
            }
            let action = s.ids.next_id();
            for a in &mut d.accounts {
                if a.ledger.start > *date {
                    continue;
                }
                let mut prior = a.ledger.clone();
                prior
                    .events
                    .retain(|e| e.date < *date || (e.date == *date && e.order < *order));
                let state = replay(&prior, *date).map_err(engine)?;
                if !state.lots.iter().any(|l| l.instrument_id == *instrument_id) {
                    continue;
                }
                if a.ledger.events.iter().any(|e|e.date==*date&&matches!(&e.kind,EventKind::Split{instrument_id:id,..} if id==instrument_id)){return Err(conflict("split already recorded on this date"))}
                a.ledger.events.push(Event {
                    id: s.ids.next_id(),
                    date: *date,
                    order: *order,
                    kind: EventKind::Split {
                        instrument_id: instrument_id.clone(),
                        numerator: *numerator,
                        denominator: *denominator,
                        action_id: action.clone(),
                    },
                });
                a.revision = revision;
                receipt.account_ids.push(a.id.clone());
            }
            if receipt.account_ids.is_empty() {
                return Err(invalid(
                    "no account holds this instrument on the split date",
                ));
            }
        }
        PortfolioMutation::ReplaceSplit {
            action_id,
            date,
            order,
            numerator,
            denominator,
        } => {
            crate::portfolio::id(action_id)?;
            if *numerator == 0 || *denominator == 0 {
                return Err(invalid("split ratio must be positive"));
            }
            for a in &mut d.accounts {
                let mut changed = false;
                for e in &mut a.ledger.events {
                    if let EventKind::Split {
                        action_id: id,
                        numerator: n,
                        denominator: den,
                        ..
                    } = &mut e.kind
                        && id == action_id
                    {
                        e.date = *date;
                        e.order = *order;
                        *n = *numerator;
                        *den = *denominator;
                        changed = true;
                    }
                }
                if changed {
                    a.revision = revision;
                    receipt.account_ids.push(a.id.clone());
                }
            }
            if receipt.account_ids.is_empty() {
                return Err(invalid("split action not found"));
            }
        }
        PortfolioMutation::VoidSplit { action_id } => {
            crate::portfolio::id(action_id)?;
            for a in &mut d.accounts {
                let before = a.ledger.events.len();
                a.ledger.events.retain(
                    |e| !matches!(&e.kind,EventKind::Split{action_id:id,..} if id==action_id),
                );
                if a.ledger.events.len() != before {
                    a.revision = revision;
                    receipt.account_ids.push(a.id.clone());
                }
            }
            if receipt.account_ids.is_empty() {
                return Err(invalid("split action not found"));
            }
        }
        PortfolioMutation::BindInstrument {
            instrument_id,
            instance_id,
            native_id,
        } => {
            crate::portfolio::id(instance_id)?;
            crate::portfolio::id(&native_id.namespace)?;
            crate::portfolio::id(&native_id.value)?;
            instrument(&mut d, instrument_id)?.binding = Some(PortfolioBinding {
                instance_id: instance_id.clone(),
                native_id: native_id.clone(),
            });
        }
        PortfolioMutation::UnbindInstrument { instrument_id } => {
            instrument(&mut d, instrument_id)?.binding = None;
        }
    }
    let today = s.clock.now().date_naive();
    for a in &d.accounts {
        for event in &a.ledger.events {
            let was_current = original
                .accounts
                .iter()
                .any(|old| old.id == a.id && old.ledger.events.iter().any(|e| e.id == event.id));
            if !was_current {
                let reserved:bool=s.connection.query_row("SELECT EXISTS(SELECT 1 FROM portfolio_event_ids WHERE account_id=?1 AND event_id=?2)",params![a.id,event.id],|r|r.get(0)).map_err(storage)?;
                if reserved {
                    return Err(conflict("voided transaction ID cannot be reused"));
                }
            }
        }

        if a.ledger.start > today || a.ledger.events.iter().any(|e| e.date > today) {
            return Err(invalid(
                "future transactions and opening dates are unsupported",
            ));
        }
        let known = |id: &str| d.instruments.iter().any(|i| i.id == id);
        if a.ledger
            .events
            .iter()
            .any(|e| e.kind.instrument_id().is_some_and(|id| !known(id)))
        {
            return Err(scoped(
                "transaction instrument does not belong to portfolio",
            ));
        }
        if let Opening::Existing { lots, .. } = &a.ledger.opening
            && lots.iter().any(|l| !known(&l.instrument_id))
        {
            return Err(scoped("opening instrument does not belong to portfolio"));
        }
        replay(&a.ledger, today).map_err(engine)?;
    }
    d.header.revision = revision;
    bound(&d, 16 * 1024 * 1024)?;
    Ok((d, receipt))
}
pub(super) fn execute(
    s: &mut SqliteApplicationStore,
    r: &PortfolioCommand,
) -> Result<PortfolioReceipt> {
    let input = bound(r, 1024 * 1024)?;
    // BEGIN IMMEDIATE excludes concurrent writers across separate application processes.
    let tx = rusqlite::Transaction::new_unchecked(
        &s.connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(storage)?;

    let prior: Option<(String, String)> = tx
        .query_row(
            "SELECT input,receipt FROM portfolio_requests WHERE id=?1",
            [&r.request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((original, receipt)) = prior {
        if original != input {
            return Err(conflict("request ID reused with different input"));
        }
        return serde_json::from_str(&receipt).map_err(storage);
    }
    let (d, receipt) = propose(s, r)?;
    let payload = bound(&d, 16 * 1024 * 1024)?;
    let audit = AuditEntry {
        request_id: r.request_id.clone(),
        revision: receipt.revision,
        recorded_at: s.clock.now(),
        mutation: r.mutation.clone(),
    };
    tx.execute("INSERT INTO portfolios(id,revision,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",params![d.header.id,receipt.revision as i64,payload]).map_err(storage)?;
    tx.execute(
        "INSERT INTO portfolio_history(portfolio_id,revision,payload,audit) VALUES(?1,?2,?3,?4)",
        params![
            d.header.id,
            receipt.revision as i64,
            payload,
            bound(&audit, 2 * 1024 * 1024)?
        ],
    )
    .map_err(storage)?;
    tx.execute(
        "INSERT INTO portfolio_requests(id,input,receipt) VALUES(?1,?2,?3)",
        params![r.request_id, input, bound(&receipt, 1024 * 1024)?],
    )
    .map_err(storage)?;
    for a in &d.accounts {
        for event in &a.ledger.events {
            tx.execute(
                "INSERT OR IGNORE INTO portfolio_event_ids(account_id,event_id) VALUES(?1,?2)",
                params![a.id, event.id],
            )
            .map_err(storage)?;
        }
    }
    tx.commit().map_err(storage)?;
    Ok(receipt)
}
