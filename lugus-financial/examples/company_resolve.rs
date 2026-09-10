//! SEC-first discovery and offline catalog inspection; no market-symbol inference.
use lugus_financial::{
    plugin::{Limits, Manifest, Plugin},
    resolution::{application::*, catalog::*, *},
    storage::SqliteRepository,
};
use serde_json::json;

const USAGE: &str = "company_resolve resolve DB MANIFEST INPUT | lookup DB MANIFEST CIK | query DB INPUT | history DB CATALOG_ID | select DB RUN_ID OBSERVATION_ID | selection DB SELECTION_ID";
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(args.len() >= 3, "{USAGE}");
    let mut repo = SqliteRepository::open(&args[1])?;
    match args[0].as_str() {
        "query" => {
            anyhow::ensure!(args.len() == 3, "{USAGE}");
            let parsed = parse_input(&args[2])?;
            let mut entries = repo.search_catalog(&parsed.primary)?;
            if entries.is_empty()
                && let Some(query) = parsed.fallback
            {
                entries = repo.search_catalog(&query)?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"scope":"local catalog history only","candidates":entries})
                )?
            );
        }
        "select" => {
            anyhow::ensure!(args.len() == 4, "{USAGE}");
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &repo.select_candidate(args[2].parse()?, args[3].parse()?)?
                )?
            );
        }
        "selection" => {
            anyhow::ensure!(args.len() == 3, "{USAGE}");
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &repo.catalog_selection(CatalogSelectionId(args[2].parse()?))?
                )?
            );
        }
        "history" => {
            anyhow::ensure!(args.len() == 3, "{USAGE}");
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &repo.catalog_history(CatalogCompanyId(args[2].parse()?))?
                )?
            );
        }
        "resolve" | "lookup" => {
            anyhow::ensure!(args.len() == 4, "{USAGE}");
            let (manifest, directory) = Manifest::load(&args[2])?;
            let instance = format!("{}:local", manifest.id);
            let config = std::env::var("LUGUS_SEC_USER_AGENT")
                .map_or(json!({}), |user_agent| json!({"user_agent":user_agent}));
            let mut plugin =
                Plugin::start(manifest, directory, instance, config, Limits::default()).await?;
            let result = if args[0] == "resolve" {
                resolve_input(&mut repo, &mut plugin, &args[3], 100, 100).await
            } else {
                match normalize_cik(&args[3]) {
                    Ok(value) => {
                        lookup_and_store(
                            &mut repo,
                            &mut plugin,
                            &LookupRequest {
                                identifier: lugus_financial::domain::CompanyId {
                                    namespace: "sec:cik".into(),
                                    value,
                                },
                            },
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            };
            let close = plugin.close().await;
            let result = result?;
            close?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            anyhow::ensure!(
                !matches!(result.outcome, ResolutionOutcome::Incomplete { .. }),
                "resolution incomplete; persisted run contains details"
            );
        }
        _ => anyhow::bail!("{USAGE}"),
    }
    Ok(())
}
