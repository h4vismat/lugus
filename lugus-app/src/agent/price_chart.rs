//! One user-level chart request, composed from the existing supervised operations.
use super::*;
use chrono::NaiveDate;
use lugus_financial::{market_data::InstrumentId, selection::PriceSeries};

pub(super) const NAME: &str = "get_price_chart";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    symbol: String,
    start: NaiveDate,
    end: NaiveDate,
}

// Provider dialects stay here, outside the public tool contract. Additional
// adapters can translate the same request without making the model build queries.
fn native_namespace(identity: &ProviderIdentity) -> Option<&'static str> {
    match (
        identity.plugin_id.as_str(),
        identity.plugin_version.as_str(),
    ) {
        ("yfinance", "0.2.0" | "0.3.0") => Some("yahoo:symbol"),
        _ => None,
    }
}
fn providers(offering: &Offering) -> impl Iterator<Item = (&ProviderIdentity, &'static str)> {
    offering
        .operations()
        .filter(|o| o.operation == Operation::Prices)
        .filter_map(|o| native_namespace(&o.identity).map(|namespace| (&o.identity, namespace)))
}
pub(super) fn tool_spec(offering: &Offering) -> Option<ToolSpec> {
    providers(offering).next()?;
    Some(ToolSpec {
        name: NAME.into(),
        description: "Get and open a saved daily closing-price chart for an explicit market symbol and inclusive date range. The backend chooses the configured supported provider, constructs its native identifier, fetches all bounded pages, saves the dataset, and requests the chart view. No namespace, provider, dataset, or view construction is needed. Use an exact listing symbol (including its market suffix where needed); ask for clarification if the listing is ambiguous. This symbol chart does not establish company identity or associate fundamentals. Multiple eligible providers require configuration disambiguation. Success means the view was accepted, not that presentation has been confirmed.".into(),
        input_schema: object(vec![
            ("symbol", json!({"type":"string","minLength":1,"maxLength":128,"description":"Exact market symbol, for example AAPL, PLTR, or 7203.T. No identifier namespace."})),
            ("start", json!({"type":"string","format":"date"})),
            ("end", json!({"type":"string","format":"date"})),
        ]),
    })
}
impl ResearchExecutor {
    fn chart_not_cancelled(&self) -> Result<()> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|c| *c.borrow() || c.has_changed().is_err())
        {
            return Err(AppError::new(
                ErrorKind::Cancelled,
                "chart request cancelled",
                false,
            ));
        }
        Ok(())
    }
    pub(super) async fn price_chart_call(
        &self,
        scope: &Scope,
        call: &ToolCall,
    ) -> Result<(bool, Value)> {
        let request: Request = serde_json::from_value(call.arguments.clone())
            .map_err(|_| invalid("expected symbol, start and end (YYYY-MM-DD)"))?;
        if request.symbol.is_empty()
            || request.symbol.len() > 128
            || request
                .symbol
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || request.start > request.end
        {
            return Err(invalid(
                "provide an explicit symbol without whitespace and an ordered date range",
            ));
        }
        let mut eligible = providers(&self.offering);
        let (provider, namespace) = eligible.next().ok_or_else(|| {
            AppError::new(
                ErrorKind::Unsupported,
                "no supported price-chart provider was offered",
                false,
            )
        })?;
        if eligible.next().is_some() {
            return Err(AppError::new(
                ErrorKind::AmbiguousProvider,
                "multiple price-chart providers are active; configure a single provider before requesting a chart",
                false,
            ));
        }
        self.chart_not_cancelled()?;
        let query = PriceQuery {
            instrument: InstrumentId {
                namespace: namespace.into(),
                value: request.symbol,
            },
            start: request.start,
            end: request.end,
            cursor: None,
            page_size: 100,
        };
        let command = FetchCommand::Prices {
            instance_id: provider.instance_id.clone(),
            query: query.clone(),
        };
        let receipt = self.application.submit(scope, &self.offering, command)?;
        let (success, status) = self.await_job(scope, receipt).await?;
        if !success {
            return Ok((false, status));
        }
        self.chart_not_cancelled()?;
        let status: JobStatus =
            serde_json::from_value(status).map_err(|_| invalid("invalid chart job status"))?;
        let fetch_id = status
            .fetch_id
            .ok_or_else(|| invalid("successful chart fetch has no receipt"))?;
        let fetch = self.application.read_fetch(scope, &fetch_id).await?;
        let run_id = fetch
            .runs
            .iter()
            .find(|r| r.kind == RunKind::Market)
            .ok_or_else(|| {
                AppError::new(
                    ErrorKind::MissingData,
                    "price fetch has no saved market run",
                    false,
                )
            })?
            .id;
        self.chart_not_cancelled()?;
        let dataset = self
            .application
            .create_dataset(
                scope,
                &fetch_id,
                DatasetProjection::Prices {
                    run_id,
                    query,
                    series: PriceSeries::Close,
                },
            )
            .await?;
        self.chart_not_cancelled()?;
        let view = self
            .application
            .open_view(
                scope,
                OpenViewRequest {
                    dataset_id: dataset.id.clone(),
                    kind: ViewKind::PriceChart,
                },
            )
            .await?;
        Ok((
            true,
            self.value(json!({
                "fetch_id":fetch_id,"dataset_id":dataset.id,"view_id":view.id,
                "row_count":dataset.row_count,"provider":dataset.provider,
                "coverage":dataset.coverage,"limitations":dataset.limitations,
                "company_binding_id":dataset.binding_id,"view_status":"accepted"
            }))?,
        ))
    }
}
