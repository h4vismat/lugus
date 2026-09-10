//! Inspect stored selections without fetching data.
//! cargo run -p lugus-financial --example observation_select -- DB INSTANCE PLUGIN VERSION NAMESPACE SYMBOL START END
use lugus_financial::{
    domain::ProviderIdentity,
    market_data::{InstrumentId, PriceQuery},
    selection::{PriceSeries, SelectionRepository, select_daily},
    storage::SqliteRepository,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 8 {
        return Err(
            "usage: observation_select DB INSTANCE PLUGIN VERSION NAMESPACE SYMBOL START END"
                .into(),
        );
    }
    let repo = SqliteRepository::open(&args[0])?;
    let provider = ProviderIdentity {
        instance_id: args[1].clone(),
        plugin_id: args[2].clone(),
        plugin_version: args[3].clone(),
    };
    let query = PriceQuery {
        instrument: InstrumentId {
            namespace: args[4].clone(),
            value: args[5].clone(),
        },
        start: args[6].parse()?,
        end: args[7].parse()?,
        cursor: None,
        page_size: 100,
    };
    let result = select_daily(
        &provider,
        &query,
        PriceSeries::Close,
        &repo.market_runs(&provider)?,
        None,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
