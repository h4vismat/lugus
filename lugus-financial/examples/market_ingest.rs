//! Explicit daily-price refresh and offline query using a selected plugin manifest.
use lugus_financial::{
    application::market::ingest_prices,
    domain::{ProviderIdentity, Validate},
    market_data::{InstrumentId, PriceQuery},
    plugin::{Limits, Manifest, Plugin},
    storage::{SqliteRepository, market::MarketRepository},
};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        (6..=7).contains(&args.len()),
        "usage: market_ingest <ingest|query> DB MANIFEST SYMBOL START END [INSTANCE]"
    );
    let mode = args[0].as_str();
    anyhow::ensure!(["ingest", "query"].contains(&mode), "unknown mode");
    let (manifest, directory) = Manifest::load(&args[2])?;
    let instance = args
        .get(6)
        .cloned()
        .unwrap_or_else(|| "yfinance-local".into());
    let identity = ProviderIdentity {
        instance_id: instance.clone(),
        plugin_id: manifest.id.clone(),
        plugin_version: manifest.version.clone(),
    };
    let query = PriceQuery {
        instrument: InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: args[3].clone(),
        },
        start: args[4].parse()?,
        end: args[5].parse()?,
        cursor: None,
        page_size: 100,
    };
    query.validate()?;
    let mut repository = SqliteRepository::open(&args[1])?;
    if mode == "query" {
        println!(
            "{}",
            serde_json::to_string_pretty(&repository.market_snapshot(&identity, &query)?)?
        );
        return Ok(());
    }
    let mut plugin =
        Plugin::start(manifest, directory, instance, json!({}), Limits::default()).await?;
    let outcome = ingest_prices(&mut repository, &mut plugin, &query).await;
    let closed = plugin.close().await;
    let run = outcome?;
    closed?;
    println!("{}", serde_json::to_string_pretty(&json!({"run_id": run}))?);
    Ok(())
}
