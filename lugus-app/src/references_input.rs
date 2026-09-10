use crate::{DatasetProjection, FetchCommand, PriceQuery, Query};
use chrono::NaiveDate;
use lugus_financial::{
    domain::Period,
    selection::{MetricQuery, PeriodSelection, PriceSeries},
};
use serde::{Deserialize, Deserializer};
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StrictProjection {
    Prices {
        run_id: i64,
        #[serde(deserialize_with = "price_query")]
        query: PriceQuery,
        series: PriceSeries,
    },
    Facts {
        run_id: i64,
        query: StrictMetric,
    },
    Filings {
        run_id: i64,
    },
    Resolution {
        run_id: i64,
    },
    Document,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictMetric {
    #[serde(deserialize_with = "financial_query")]
    scope: Query,
    namespace: String,
    concept: String,
    unit: String,
    periods: StrictPeriods,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StrictPeriods {
    LatestInstant,
    Instants,
    Durations,
    Exact { period: StrictPeriod },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StrictPeriod {
    Instant { date: NaiveDate },
    Duration { start: NaiveDate, end: NaiveDate },
}
fn price_query<'de, D: Deserializer<'de>>(d: D) -> Result<PriceQuery, D::Error> {
    let query = serde_json::Value::deserialize(d)?;
    match serde_json::from_value::<FetchCommand>(
        serde_json::json!({"operation":"prices","instance_id":"projection","query":query}),
    )
    .map_err(serde::de::Error::custom)?
    {
        FetchCommand::Prices { query, .. } => Ok(query),
        _ => unreachable!(),
    }
}
fn financial_query<'de, D: Deserializer<'de>>(d: D) -> Result<Query, D::Error> {
    let query = serde_json::Value::deserialize(d)?;
    match serde_json::from_value::<FetchCommand>(
        serde_json::json!({"operation":"facts","instance_id":"projection","query":query}),
    )
    .map_err(serde::de::Error::custom)?
    {
        FetchCommand::Facts { query, .. } => Ok(query),
        _ => unreachable!(),
    }
}
impl<'de> Deserialize<'de> for DatasetProjection {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match StrictProjection::deserialize(d)? {
            StrictProjection::Prices {
                run_id,
                query,
                series,
            } => Self::Prices {
                run_id,
                query,
                series,
            },
            StrictProjection::Facts { run_id, query: q } => Self::Facts {
                run_id,
                query: MetricQuery {
                    scope: q.scope,
                    namespace: q.namespace,
                    concept: q.concept,
                    unit: q.unit,
                    periods: match q.periods {
                        StrictPeriods::LatestInstant => PeriodSelection::LatestInstant,
                        StrictPeriods::Instants => PeriodSelection::Instants,
                        StrictPeriods::Durations => PeriodSelection::Durations,
                        StrictPeriods::Exact { period } => PeriodSelection::Exact {
                            period: match period {
                                StrictPeriod::Instant { date } => Period::Instant { date },
                                StrictPeriod::Duration { start, end } => {
                                    Period::Duration { start, end }
                                }
                            },
                        },
                    },
                },
            },
            StrictProjection::Filings { run_id } => Self::Filings { run_id },
            StrictProjection::Resolution { run_id } => Self::Resolution { run_id },
            StrictProjection::Document => Self::Document,
        })
    }
}
