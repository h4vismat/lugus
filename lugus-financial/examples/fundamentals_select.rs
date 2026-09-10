//! Inspect reported-fact selections from local evidence only.
use lugus_financial::{
    domain::{CompanyId, ProviderIdentity, Query},
    resolution::normalize_cik,
    selection::{MetricQuery, PeriodSelection, SelectionRepository, select_facts},
    storage::SqliteRepository,
};
fn main() -> anyhow::Result<()> {
    let a = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(
        a.len() == 11,
        "usage: fundamentals_select DB INSTANCE PLUGIN VERSION CIK NAMESPACE CONCEPT UNIT FILED_FROM FILED_TO latest-instant|instants|durations"
    );
    let repository = SqliteRepository::open(&a[0])?;
    let provider = ProviderIdentity {
        instance_id: a[1].clone(),
        plugin_id: a[2].clone(),
        plugin_version: a[3].clone(),
    };
    let periods = match a[10].as_str() {
        "latest-instant" => PeriodSelection::LatestInstant,
        "instants" => PeriodSelection::Instants,
        "durations" => PeriodSelection::Durations,
        _ => anyhow::bail!("invalid period selection"),
    };
    let query = MetricQuery {
        scope: Query {
            company: CompanyId {
                namespace: "sec:cik".into(),
                value: normalize_cik(&a[4])?,
            },
            filed_from: a[8].parse()?,
            filed_to: a[9].parse()?,
            forms: vec![],
            cursor: None,
            page_size: 100,
        },
        namespace: a[5].clone(),
        concept: a[6].clone(),
        unit: a[7].clone(),
        periods,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&select_facts(
            &provider,
            &query,
            &repository.financial_runs(&provider)?,
            None
        )?)?
    );
    Ok(())
}
