//! Typed, bounded native transport for portfolio operations independent of chat execution.
use lugus_app::{portfolio::*, *};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    List {
        #[serde(default)]
        offset: usize,
    },
    Execute {
        request: PortfolioCommand,
    },
    Preview {
        request: PortfolioCommand,
    },
    Overview {
        portfolio_id: String,
        account_id: Option<String>,
    },
    Rows {
        portfolio_id: String,
        account_id: Option<String>,
        section: String,
        offset: usize,
        revision: Option<String>,
    },
    Audit {
        portfolio_id: String,
        offset: usize,
    },
    Refresh {
        request: RefreshRequest,
    },
    RefreshStatus {
        id: String,
    },
    Snapshot {
        request: SnapshotRequest,
    },
    RefreshCancel {
        id: String,
    },
    Providers,
}
pub(crate) async fn dispatch(app: &Application, c: Command) -> Result<Value> {
    match c {
        Command::List { offset } => {
            bounded_page(app, |limit| async move {
                super::value(app.portfolio_list(PageRequest { offset, limit }).await?)
            })
            .await
        }
        Command::Execute { request } => super::value(app.execute_portfolio(request).await?),
        Command::Preview { request } => super::value(app.preview_portfolio(request).await?),
        Command::Overview {
            portfolio_id,
            account_id,
        } => super::value(app.portfolio_overview(portfolio_id, account_id).await?),
        Command::Rows {
            portfolio_id,
            account_id,
            section,
            offset,
            revision,
        } => {
            let revision = revision
                .map(|r| r.parse::<u64>())
                .transpose()
                .map_err(|_| super::error(ErrorKind::InvalidInput, "invalid revision"))?;
            bounded_page(app, |limit| {
                let (portfolio_id, account_id, section) =
                    (portfolio_id.clone(), account_id.clone(), section.clone());
                async move {
                    super::value(
                        app.portfolio_rows(
                            portfolio_id,
                            account_id,
                            section,
                            PageRequest { offset, limit },
                            revision,
                        )
                        .await?,
                    )
                }
            })
            .await
        }
        Command::Audit {
            portfolio_id,
            offset,
        } => {
            bounded_page(app, |limit| {
                let portfolio_id = portfolio_id.clone();
                async move {
                    super::value(
                        app.portfolio_audit(portfolio_id, PageRequest { offset, limit })
                            .await?,
                    )
                }
            })
            .await
        }

        Command::Refresh { request } => super::value(app.refresh_portfolio(request).await?),
        Command::RefreshStatus { id } => super::value(app.portfolio_refresh_status(id).await?),
        Command::Snapshot { request } => {
            super::value(app.create_portfolio_snapshot(request).await?)
        }
        Command::RefreshCancel { id } => {
            app.cancel_portfolio_refresh(id)?;
            super::value(serde_json::json!({"cancelled":true}))
        }
        Command::Providers => {
            let offering = app.offering()?;
            let entries = offering
                .operations()
                .filter(|o| o.operation == Operation::Prices)
                .map(|o| json!({"instance_id":o.identity.instance_id,"name":o.identity.plugin_id}))
                .collect::<Vec<_>>();
            super::value(entries)
        }
    }
}

// Reduce the page before crossing the configured native response budget. Offsets
// come from the store's returned page, so smaller pages cannot skip records.
async fn bounded_page<F, Fut>(app: &Application, mut fetch: F) -> Result<Value>
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = Result<Value>>,
{
    let mut limit = 100;
    loop {
        let result = fetch(limit).await.and_then(|value| {
            lugus_app::agent_contract::check_serialized_size(
                &value,
                super::MAX_BYTES.min(app.limits().max_output_bytes),
            )?;
            Ok(value)
        });
        match result {
            Err(e) if e.kind == ErrorKind::ResourceLimit && limit > 1 => limit = (limit / 2).max(1),
            result => return result,
        }
    }
}
