//! Synthetic live verification; no application or workspace data is supplied.
use lugus_agent::{
    AgentRuntime, RunLimits, RunRequest, RunSubject, ToolCall, ToolExecutor, ToolResult, ToolSpec,
    claude::{ClaudeConfig, ClaudeRuntime},
};
use std::{path::PathBuf, time::Duration};
use tokio::sync::{mpsc, watch};
struct SyntheticTools;
#[async_trait::async_trait]
impl ToolExecutor for SyntheticTools {
    async fn execute(&self, call: ToolCall) -> ToolResult {
        ToolResult {
            success: call.name == "synthetic_value",
            content: "Synthetic verification value: 42. This is fictional test data.".into(),
        }
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let workspace = args
        .next()
        .ok_or("usage: claude_session WORKSPACE [CLAUDE_EXECUTABLE] [MODEL]")?;
    let executable = args.next().unwrap_or_else(|| "claude".into());
    let model = args.next();
    let mut runtime = ClaudeRuntime::connect(ClaudeConfig {
        executable: PathBuf::from(executable),
        workspace: PathBuf::from(workspace),
        model,
    })
    .await?;
    let (events, mut receive) = mpsc::channel(32);
    let printer = tokio::spawn(async move {
        while let Some(event) = receive.recv().await {
            println!("{event:?}");
        }
    });
    let (_cancel, cancellation) = watch::channel(false);
    let request = RunRequest {
        run_id: "claude-synthetic-verification".into(), subject: RunSubject::Conversation { id: "synthetic".into() },
        instructions: "You are a synthetic integration test assistant. Call the synthetic_value tool exactly once and report its fictional value. Do not read any workspace files.".into(),
        context: "No real application data is involved.".into(), prompt: "Call synthetic_value and report the result in one sentence.".into(), allow_web_search: false,
        tools: vec![ToolSpec { name: "synthetic_value".into(), description: "Returns a fictional number for integration verification".into(), input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}) }],
        limits: RunLimits { timeout: Duration::from_secs(90), max_tool_calls: 1, max_tool_result_bytes: 1024 },
    };
    let report = runtime
        .run(request, &SyntheticTools, events, cancellation)
        .await;
    runtime.close().await?;
    printer.await?;
    println!("{:?}", report?);
    Ok(())
}
