//! Explicit local workflow. Run without arguments for command syntax.
use lugus_agent::{
    RunLimits,
    codex::{CodexConfig, CodexRuntime},
    reviews::*,
};
use lugus_financial::{
    domain::{ProviderIdentity, Query},
    storage::SqliteRepository,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::{collections::HashSet, error::Error, fs, path::Path, time::Duration};
use tokio::sync::{mpsc, watch};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const USAGE: &str = "Commands:
  create AGENT_DB THESIS_ID TEXT_FILE
  edit AGENT_DB THESIS_ID EXPECTED_REVISION TEXT_FILE
  show AGENT_DB THESIS_ID
  status AGENT_DB REVIEW_ID
  evidence FINANCIAL_DB PROVIDER_JSON QUERY_JSON
  queue AGENT_DB REVIEW_ID THESIS_ID REVISION FINANCIAL_DB PROVIDER_JSON QUERY_JSON [SELECTED_IDS_JSON]
  run AGENT_DB REVIEW_ID EMPTY_WORKSPACE [CANCEL_AFTER_MS]
  recover AGENT_DB

Provider/query arguments are file paths containing the existing financial API JSON types.
The optional selection file is a JSON array of evidence IDs; snapshot scope is always included.
Only 'run' contacts the configured model. Run uses codex-cli 0.153.4 and existing Codex authentication.
'recover' is startup recovery: stop all active review executors before invoking it.";

fn read_json<T: DeserializeOwned>(path: &str) -> Result<T> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}
fn print(value: impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn capture(db: &str, provider: &str, query: &str) -> Result<Vec<Evidence>> {
    if !Path::new(db).is_file() {
        return Err("financial database does not exist".into());
    }
    let provider: ProviderIdentity = read_json(provider)?;
    let query: Query = read_json(query)?;
    Ok(capture_financial_evidence(
        &SqliteRepository::open(db)?,
        &provider,
        &query,
    )?)
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["create", db, id, file] => print(SqliteReviewStore::open(db)?.save_thesis(
            id,
            0,
            &fs::read_to_string(file)?,
            SystemClock.now(),
        )?),
        ["edit", db, id, revision, file] => print(SqliteReviewStore::open(db)?.save_thesis(
            id,
            revision.parse()?,
            &fs::read_to_string(file)?,
            SystemClock.now(),
        )?),
        ["show", db, id] => {
            let store = SqliteReviewStore::open(db)?;
            let assessments = store.assessments(id)?;
            let reviews = assessments
                .iter()
                .map(|a| store.review(&a.review_id))
                .collect::<ReviewResult<Vec<_>>>()?;
            print(
                json!({"thesis":store.thesis(id,None)?,"assessments":assessments,"assessed_reviews":reviews}),
            )
        }
        ["status", db, id] => print(SqliteReviewStore::open(db)?.review(id)?),
        ["evidence", db, provider, query] => print(capture(db, provider, query)?),
        [
            "queue",
            db,
            id,
            thesis,
            revision,
            financial,
            provider,
            query,
            selection @ ..,
        ] if selection.len() <= 1 => {
            let mut items = capture(financial, provider, query)?;
            if let Some(file) = selection.first() {
                let selected: Vec<String> = read_json(file)?;
                let ids: HashSet<_> = selected.iter().map(String::as_str).collect();
                if ids.len() != selected.len()
                    || ids.iter().any(|id| !items.iter().any(|e| e.id == *id))
                {
                    return Err("selection contains duplicate or unknown evidence IDs".into());
                }
                items.retain(|e| {
                    e.content["kind"] == "snapshot_scope" || ids.contains(e.id.as_str())
                });
            }
            print(SqliteReviewStore::open(db)?.enqueue(
                id,
                thesis,
                revision.parse()?,
                items,
                SystemClock.now(),
            )?)
        }
        ["recover", db] => print(
            json!({"interrupted_reviews":SqliteReviewStore::open(db)?.recover(SystemClock.now())?}),
        ),
        ["run", db, id, workspace, cancel_ms @ ..] if cancel_ms.len() <= 1 => {
            let delay = cancel_ms.first().map(|v| v.parse::<u64>()).transpose()?;
            if !Path::new(workspace).is_dir() || fs::read_dir(workspace)?.next().is_some() {
                return Err("run requires an existing empty dedicated workspace".into());
            }
            let store = SqliteReviewStore::open(db)?;
            let review = store.review(id)?;
            if review.status == ReviewStatus::Completed {
                return print(review);
            }
            let mut runtime = CodexRuntime::connect(CodexConfig {
                executable: "codex".into(),
                workspace: workspace.into(),
                model: None,
                model_provider: None,
            })
            .await?;
            let (events, mut receiver) = mpsc::channel(64);
            let display = tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());
                }
            });
            let (sender, cancel) = watch::channel(false);
            let cancellation = tokio::spawn(async move {
                if let Some(ms) = delay {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    let _ = sender.send(true);
                    std::future::pending::<()>().await;
                } else {
                    let _keep_sender = sender;
                    std::future::pending::<()>().await;
                }
            });
            let result = ReviewCoordinator::new(&store, &SystemClock)
                .execute(
                    &mut runtime,
                    ReviewExecution {
                        review_id: (*id).into(),
                        runtime_identity: "codex-cli:0.153.4; configured model/provider".into(),
                        limits: RunLimits {
                            timeout: Duration::from_secs(300),
                            max_tool_calls: 80,
                            max_tool_result_bytes: 131072,
                        },
                    },
                    events,
                    cancel,
                )
                .await;
            cancellation.abort();
            let _ = cancellation.await;
            let _ = display.await;
            let review = result?;
            print(&review)?;
            if review.status != ReviewStatus::Completed {
                return Err("review did not complete; inspect its persisted status above".into());
            }
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            Err("invalid command arguments".into())
        }
    }
}
