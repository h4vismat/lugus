# lugus-agent

`lugus-agent` is the first-milestone runtime harness for disposable agent turns. Its public `AgentRuntime` and `ToolExecutor` boundaries do not expose Codex protocol types, so another runtime adapter can implement the same contracts. The current adapter supports `codex-cli 0.153.4` exactly.

## Run the Codex example

Install that Codex version separately and authenticate with Codex itself:

```sh
codex --version
codex login status
```

The harness does not start login, read tokens, copy credentials, or store account identity. A normal Codex installation can be authenticated with `codex login` before running the example.

Create a dedicated working directory that contains only files the demonstration may expose to its configured model provider. The adapter starts Codex with a read-only sandbox, disables inherited MCP servers and unrelated execution features, and does not modify global Codex configuration.

```sh
mkdir -p /tmp/lugus-codex-demo
cargo run -p lugus-agent --example codex_session -- \
  /tmp/lugus-codex-demo \
  'Call lugus_context once, repeat its returned content, then explain which statement is demonstration data.'
```

The command prints runtime events, the terminal outcome, and the final assistant text. `lugus_context` returns fixed, application-supplied test data through a read-only callback. It does not query or write Lugus financial storage.

Codex uses its configured model and provider by default. Explicit overrides are available for installations that define them:

```sh
cargo run -p lugus-agent --example codex_session -- \
  /tmp/lugus-codex-demo 'Call lugus_context and summarize it.' \
  --model MODEL --provider PROVIDER
```

Use `--codex PATH` when the supported executable is outside `PATH`. Even when Lugus stores data locally and coordinates execution locally, the prompt, application context, tool results, and readable workspace content can be sent to the selected remote model provider. Choose the workspace and provider accordingly.

To exercise cancellation deterministically, request a turn that will remain active and set a delay:

```sh
cargo run -p lugus-agent --example codex_session -- \
  /tmp/lugus-codex-demo 'Continue reasoning until interrupted.' \
  --cancel-after-ms 500
```

Cancellation sends Codex `turn/interrupt`, waits for a bounded grace period, and reaps the disposable process. A cancelled adapter instance is not reused.

## Runtime boundary

One `CodexRuntime` adapter instance runs one task at a time. The harness returns runtime completion independently of future assessment validation. It does not store memories, persist an assessment, schedule reviews, capture documents, or turn native web-search output into durable evidence. Those are later application and storage milestones.

Native web search is requested in the read-only Codex configuration, but actual availability depends on the selected model and provider. A public-document lookup can demonstrate that capability; its response remains transient. Capturing source documents and provenance belongs to milestone 3.

Tool lifecycle events describe adapter handling of an accepted flat call. `ToolStarted` is emitted before dispatch, so an unknown flat tool can produce `ToolStarted` and an unsuccessful `ToolFinished` without invoking the application executor. Interruption or a terminal failure after `ToolStarted` can prevent `ToolFinished`; the terminal run result is authoritative. A call rejected by a host limit emits neither lifecycle event because tool execution never began, and the run returns an explicit terminal error.

## Deterministic verification

From the workspace root:

```sh
cargo test -p lugus-agent
cargo clippy -p lugus-agent --all-targets -- -D warnings
cargo fmt -p lugus-agent -- --check
cargo test -p lugus-financial
```

These checks use a local protocol fixture and do not establish that a configured account, provider, or native web search works live.

## Live verification record

On 2026-09-10, the example was run with the user's existing authenticated Codex configuration and its default model/provider. No account identity or credential was captured.

- A fresh empty workspace run called `lugus_context` exactly once. It delivered `Started`, `ToolStarted`, `ToolFinished { success: true }`, text deltas, and a completed report. The final text included the complete fixed demonstration result.
- A separate fresh empty workspace run requested a native lookup of the official Rust `std::time::Duration` documentation. Sanitized protocol metadata recorded `item/started` with `item_type=webSearch`, followed by `item/completed` with `item_type=webSearch` and `action_type=search`. The turn returned the canonical page title and `https://doc.rust-lang.org/std/time/struct.Duration.html`, confirming that native search was available in this configuration. The response was transient and was not captured as evidence.
- A third fresh empty workspace run used `--cancel-after-ms 500`. It delivered `Started` and returned `Cancelled` without tool or text events, confirming interruption and process cleanup through the runnable interface.

The observed initialize, tool callback, text-delta, completion, and interruption shapes matched the pinned protocol fixture; no adapter change was required.
