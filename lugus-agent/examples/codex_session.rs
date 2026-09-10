use std::env;
use std::path::PathBuf;
use std::time::Duration;

use lugus_agent::codex::{AccountStatus, CodexConfig, CodexRuntime};
use lugus_agent::{
    AgentRuntime, RunLimits, RunOutcome, RunRequest, RuntimeEvent, ToolCall, ToolExecutor,
    ToolResult, ToolSpec,
};
use tokio::sync::{mpsc, watch};

const USAGE: &str = "usage: codex_session WORKSPACE PROMPT [--model MODEL] [--provider PROVIDER] [--codex PATH] [--cancel-after-ms MILLISECONDS]";

#[derive(Debug, PartialEq, Eq)]
struct CliArgs {
    workspace: PathBuf,
    prompt: String,
    model: Option<String>,
    provider: Option<String>,
    executable: PathBuf,
    cancel_after: Option<Duration>,
}

fn parse_args<I, S>(args: I) -> Result<CliArgs, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let workspace = args.next().ok_or(USAGE)?;
    let prompt = args.next().ok_or(USAGE)?;
    let mut parsed = CliArgs {
        workspace: workspace.into(),
        prompt,
        model: None,
        provider: None,
        executable: "codex".into(),
        cancel_after: None,
    };

    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--model" => parsed.model = Some(value),
            "--provider" => parsed.provider = Some(value),
            "--codex" => parsed.executable = value.into(),
            "--cancel-after-ms" => {
                let milliseconds = value
                    .parse::<u64>()
                    .map_err(|_| "--cancel-after-ms must be an integer")?;
                parsed.cancel_after = Some(Duration::from_millis(milliseconds));
            }
            _ => return Err(format!("unknown option: {flag}")),
        }
    }

    Ok(parsed)
}

struct DemonstrationTools;

#[async_trait::async_trait]
impl ToolExecutor for DemonstrationTools {
    async fn execute(&self, call: ToolCall) -> ToolResult {
        match call.name.as_str() {
            "lugus_context" => ToolResult {
                success: true,
                content: "Demonstration context: thesis A depends on financing costs. This is application-supplied test data, not market evidence.".into(),
            },
            _ => ToolResult {
                success: false,
                content: "Unknown tool".into(),
            },
        }
    }
}

fn request(prompt: String) -> RunRequest {
    RunRequest {
        run_id: "codex-session-example".into(),
        thesis_id: "demonstration-thesis-A".into(),
        instructions: "You are a demonstration research agent. Use lugus_context when the prompt asks for application context, and identify it as demonstration data.".into(),
        context: "This is a disposable runtime demonstration. Do not claim that its output is saved, validated, or durable evidence.".into(),
        prompt,
        tools: vec![ToolSpec {
            name: "lugus_context".into(),
            description: "Return read-only application-supplied demonstration context".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }],
        limits: RunLimits {
            timeout: Duration::from_secs(300),
            max_tool_calls: 4,
            max_tool_result_bytes: 4 * 1024,
        },
    }
}

async fn print_events(mut events: mpsc::Receiver<RuntimeEvent>) {
    while let Some(event) = events.recv().await {
        match event {
            RuntimeEvent::Started { run_id } => println!("event started run_id={run_id}"),
            RuntimeEvent::TextDelta { text } => println!("event text_delta {text:?}"),
            RuntimeEvent::ToolStarted { call_id, name } => {
                println!("event tool_started call_id={call_id} name={name}")
            }
            RuntimeEvent::ToolFinished { call_id, success } => {
                println!("event tool_finished call_id={call_id} success={success}")
            }
            RuntimeEvent::Usage {
                input_tokens,
                output_tokens,
            } => println!("event usage input_tokens={input_tokens} output_tokens={output_tokens}"),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args(env::args().skip(1))
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    let workspace = args.workspace.canonicalize()?;
    if !workspace.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "WORKSPACE must be a directory",
        )
        .into());
    }

    let mut runtime = CodexRuntime::connect(CodexConfig {
        executable: args.executable,
        workspace,
        model: args.model,
        model_provider: args.provider,
    })
    .await?;
    if runtime.account_status().await? != AccountStatus::Ready {
        runtime.close().await?;
        return Err(
            "Codex authentication is required; run `codex login` outside this example".into(),
        );
    }

    let (event_tx, event_rx) = mpsc::channel(32);
    let event_printer = tokio::spawn(print_events(event_rx));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let cancellation = args.cancel_after.map(|delay| {
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = cancel_tx.send(true);
        })
    });

    let result = runtime
        .run(
            request(args.prompt),
            &DemonstrationTools,
            event_tx,
            cancel_rx,
        )
        .await;
    let close_result = runtime.close().await;
    if let Some(cancellation) = cancellation {
        cancellation.abort();
    }
    event_printer.await?;

    let report = result?;
    close_result?;
    match report.outcome {
        RunOutcome::Completed => println!("outcome completed\n{}", report.final_text),
        RunOutcome::Cancelled => println!("outcome cancelled"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::parse_args;

    #[test]
    fn parses_workspace_prompt_and_optional_runtime_overrides() {
        let args = [
            "./scratch",
            "Call lugus_context",
            "--model",
            "gpt-example",
            "--provider",
            "example-provider",
            "--codex",
            "/opt/codex",
            "--cancel-after-ms",
            "250",
        ];

        let parsed = parse_args(args).unwrap();

        assert_eq!(parsed.workspace, PathBuf::from("./scratch"));
        assert_eq!(parsed.prompt, "Call lugus_context");
        assert_eq!(parsed.model.as_deref(), Some("gpt-example"));
        assert_eq!(parsed.provider.as_deref(), Some("example-provider"));
        assert_eq!(parsed.executable, PathBuf::from("/opt/codex"));
        assert_eq!(parsed.cancel_after, Some(Duration::from_millis(250)));
    }

    #[test]
    fn rejects_an_invalid_cancellation_delay() {
        let error = parse_args(["./scratch", "Wait", "--cancel-after-ms", "soon"]).unwrap_err();

        assert_eq!(error, "--cancel-after-ms must be an integer");
    }
}
