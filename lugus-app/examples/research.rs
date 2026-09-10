// See lugus-app/README.md for configuration and reproducible synthetic acceptance commands.
mod support;
use lugus_agent::{
    reviews::{ReviewStore, SqliteReviewStore},
    runtime::*,
};
use lugus_app::*;
use serde_json::{Value, json};
use std::{io::Read, time::Duration};
use tokio::sync::{mpsc, watch};
type CliResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
async fn read_file(path: &str) -> CliResult<Vec<u8>> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || -> CliResult<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err("input file exceeds 1 MiB".into());
        }
        Ok(bytes)
    })
    .await?
}
async fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> CliResult<T> {
    Ok(serde_json::from_slice(&read_file(path).await?)?)
}
fn value<T: serde::Serialize>(value: T) -> CliResult<Value> {
    Ok(serde_json::to_value(value)?)
}
async fn agent_fixture(
    app: &Application,
    workspace: &str,
    db: &str,
    thesis_id: &str,
    command_file: &str,
) -> CliResult<Value> {
    let command: FetchCommand = read_json(command_file).await?;
    let db = db.to_string();
    let thesis_id = thesis_id.to_string();
    let thesis =
        tokio::task::spawn_blocking(move || SqliteReviewStore::open(db)?.thesis(&thesis_id, None))
            .await??;
    let run = RandomIds::new()?.next_id();
    let scope = app.scope(workspace, &run, Some(&run))?;
    let tools = ResearchExecutor::new(app.clone(), scope.clone())?;
    let request = RunRequest {
        run_id: run,
        thesis_id: thesis.thesis_id.clone(),
        instructions: "Use explicit provider evidence and accept a view request.".into(),
        context: thesis.text.clone(),
        prompt: "Run the deterministic evidence workflow for this stored thesis.".into(),
        allow_web_search: false,
        tools: tools.tool_specs().to_vec(),
        limits: RunLimits {
            timeout: Duration::from_secs(120),
            max_tool_calls: 5,
            max_tool_result_bytes: app.limits().max_output_bytes,
        },
    };
    let mut runtime = support::FixtureRuntime {
        thesis: thesis.clone(),
        command,
        receipts: Value::Null,
    };
    let (events, _receiver) = mpsc::channel(16);
    let (_cancel, cancel) = watch::channel(false);
    let result = runtime.run(request, &tools, events, cancel).await;
    runtime.close().await?;
    Ok(json!({"scope":scope,"thesis":thesis,"report":result?,"receipts":runtime.receipts}))
}
async fn local(app: &Application, scope: &Scope, command: &str, args: &[&str]) -> CliResult<Value> {
    match (command, args) {
        ("fetch", [file]) => {
            let command = read_json(file).await?;
            match app.submit_manual(scope, command) {
                Ok(job) => value(app.wait(scope, &job.id).await?),
                Err(error) => Ok(json!({"scope":scope,"error":error})),
            }
        }
        ("read-fetch", [id]) => value(app.read_fetch(scope, id).await?),
        ("dataset", [id, file]) => value(
            app.create_dataset(scope, id, read_json(file).await?)
                .await?,
        ),
        ("header", [id]) => value(app.dataset_header(scope, id).await?),
        ("read", [id, offset, limit]) => value(
            app.read_dataset(
                scope,
                id,
                PageRequest {
                    offset: offset.parse()?,
                    limit: limit.parse()?,
                },
            )
            .await?,
        ),
        ("document", [id, offset, length]) => value(
            app.read_document(scope, id, offset.parse()?, length.parse()?)
                .await?,
        ),
        ("view", [id, kind]) => value(
            app.open_view(
                scope,
                OpenViewRequest {
                    dataset_id: (*id).into(),
                    kind: serde_json::from_value(json!(kind))?,
                },
            )
            .await?,
        ),
        ("read-view", [id]) => value(app.read_view(scope, id).await?),
        ("select", [id, observation]) => value(
            app.select_candidate(scope, id, observation.parse()?)
                .await?,
        ),
        _ => Err("invalid command arguments; see lugus-app/README.md".into()),
    }
}
async fn run(args: &[&str]) -> CliResult<Value> {
    if let ["thesis-create", db, id, file] = args {
        let text = String::from_utf8(read_file(file).await?)?;
        let db = db.to_string();
        let id = id.to_string();
        return value(
            tokio::task::spawn_blocking(move || {
                SqliteReviewStore::open(db)?.save_thesis(&id, 0, &text, chrono::Utc::now())
            })
            .await??,
        );
    }
    if let ["agent-fixture", config, workspace, db, thesis, file] = args {
        let app = ApplicationConfig::load(config).await?.open(false).await?;
        let result = agent_fixture(&app, workspace, db, thesis, file).await;
        let cleanup = app.shutdown().await;
        cleanup?;
        return result;
    }
    let [command, config, workspace, request, tail @ ..] = args else {
        return Err(
            "expected COMMAND CONFIG WORKSPACE REQUEST [arguments]; see lugus-app/README.md".into(),
        );
    };
    let app = ApplicationConfig::load(config)
        .await?
        .open(*command != "fetch")
        .await?;
    let result = match app.scope(workspace, request, None) {
        Ok(scope) => local(&app, &scope, command, tail).await,
        Err(error) => Err(error.into()),
    };
    let cleanup = app.shutdown().await;
    cleanup?;
    result
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    match run(&args).await {
        Ok(value) => println!(
            "{}",
            serde_json::to_string(&value).expect("JSON value serializes")
        ),
        Err(_) => {
            eprintln!(
                "research command failed; check arguments, stored thesis and configured resources"
            );
            std::process::exit(1);
        }
    }
}
