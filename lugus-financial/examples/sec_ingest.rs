//! Explicit refresh and offline read example; no background or implicit network activity.
use lugus_financial::{
    application::{ingest, retrieve_document},
    domain::{CompanyId, ProviderIdentity, Query, Validate},
    plugin::{Limits, Manifest, Plugin},
    storage::{Repository, SqliteRepository},
};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: sec_ingest <ingest|query> DB MANIFEST CIK FROM TO [INSTANCE]\n       sec_ingest document DB MANIFEST URL [INSTANCE]"
    );
    let mode = args[0].as_str();
    anyhow::ensure!(
        ["ingest", "query", "document"].contains(&mode),
        "unknown mode"
    );
    let (manifest, directory) = Manifest::load(&args[2])?;
    let instance = if mode == "document" {
        args.get(4)
    } else {
        args.get(6)
    }
    .cloned()
    .unwrap_or_else(|| "sec-local".into());
    let identity = ProviderIdentity {
        instance_id: instance.clone(),
        plugin_id: manifest.id.clone(),
        plugin_version: manifest.version.clone(),
    };
    let query = if mode != "document" {
        anyhow::ensure!(
            (6..=7).contains(&args.len()),
            "ingest/query require DB MANIFEST CIK FROM TO [INSTANCE]"
        );
        anyhow::ensure!(
            args[3].len() == 10 && args[3].bytes().all(|b| b.is_ascii_digit()),
            "supply a zero-padded 10-digit CIK"
        );
        let q = Query {
            company: CompanyId {
                namespace: "sec:cik".into(),
                value: args[3].clone(),
            },
            filed_from: args[4].parse()?,
            filed_to: args[5].parse()?,
            forms: vec![],
            cursor: None,
            page_size: 100,
        };
        q.validate()?;
        Some(q)
    } else {
        anyhow::ensure!(
            (4..=5).contains(&args.len()),
            "document requires DB MANIFEST URL [INSTANCE]"
        );
        None
    };
    let mut repository = SqliteRepository::open(&args[1])?;
    if mode == "query" {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &repository.snapshot(&identity, query.as_ref().unwrap())?
            )?
        );
        return Ok(());
    }
    let user_agent = std::env::var("LUGUS_SEC_USER_AGENT").map_err(|_| {
        anyhow::anyhow!(
            "set LUGUS_SEC_USER_AGENT to an identifying application name and contact email"
        )
    })?;
    let mut plugin = Plugin::start(
        manifest,
        directory,
        instance,
        json!({"user_agent":user_agent}),
        Limits::default(),
    )
    .await?;
    let outcome = if mode == "document" {
        retrieve_document(&mut repository, &mut plugin, &args[3], 10 * 1024 * 1024)
            .await
            .map(|checksum| json!({"checksum":checksum}))
    } else {
        ingest(&mut repository, &mut plugin, query.as_ref().unwrap())
            .await
            .map(|run| json!({"run_id":run}))
    };
    let closed = plugin.close().await;
    println!("{}", serde_json::to_string_pretty(&outcome?)?);
    closed?;
    Ok(())
}
