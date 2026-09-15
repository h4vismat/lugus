The Rust transport integration tests are in `src-tauri/tests` and run with:

    cargo test --manifest-path src-tauri/Cargo.toml --tests

`conversation.rs` injects an explicit deterministic `AgentRuntime` through
`Bridge::from_host`. Its tools call the real conversation executor and persisted
application with the synthetic JSON-RPC provider in
`lugus-desktop/tests/fixtures/provider.py`. It is not a production runtime or financial
source. The test creates both price and fundamentals views during a turn, freezes
selected evidence for a follow-up, tests scope enforcement, and reopens the real
SQLite stores offline. It also checks cancellation and failure terminal states.

Native runtime settings are external JSON, for example:

```json
{
  "application_config": "application.json",
  "runtime": {
    "executable": "/absolute/path/to/codex",
    "workspace": "runtime",
    "model": null,
    "model_provider": null,
    "timeout_secs": 180
  }
}
```

Create the runtime directory outside any Git repository. Paths are relative to
the desktop config file; the application config resolves its own paths. The
existing Codex adapter checks its exact supported version (currently 0.153.4)
for every fresh turn. It uses the installed CLI's account/provider configuration.
`timeout_secs` is optional (default 180, allowed 10–600).
Omit `runtime` to read persisted chats without starting a model. Configured financial
providers remain available; launch offline to disable provider processes too. Local creation, selection, and presentation
acknowledgement work offline; only sending requires a runtime. Production never
uses fixture answers.

For native QA, seed a new directory using actual conversation/tools/persistence:

    cargo run --manifest-path src-tauri/Cargo.toml --example seed_chat -- /tmp/lugus-qa-new

Then launch the native app with the generated `desktop.json`. This explicit
synthetic fixture creates source-supported company binding, ten daily price rows,
reported facts, a completed assistant message and two views. The example refuses
an existing target directory. No fixture runtime is linked into the native app.


`src-tauri/tests/agent_settings.rs` verifies saved agent selection, legacy configuration, unavailable/offline profiles, persistence failures and routing through the real Claude adapter. `tests/fixtures/claude.py` supplies synthetic interpretation and answer responses without credentials or network model calls. The Claude adapter binds a temporary localhost MCP endpoint, so these tests require local loopback networking.

## Portfolio dashboard

Build `npm run build` and `cargo build --manifest-path src-tauri/Cargo.toml --example portfolio_qa`, then run `node tests/portfolio-browser.cjs` and `node tests/portfolio-dashboard-browser.cjs`. The shared native QA bridge uses disposable SQLite databases; the dashboard harness supplies the existing managed-worker fixture as a historical provider. It covers chart ranges, keyboard inspection, source details, daily pages, holdings, drawer focus, 600px layout, refresh, unsent drafts, missing prices and offline reopening.

`node tests/portfolio-live-browser.cjs` is an opt-in network check with the real bundled yfinance plugin and a labelled synthetic AAPL account. Install that plugin’s pinned dependencies first. Both browser harnesses accept `LUGUS_PLAYWRIGHT_MODULE` and `LUGUS_CHROMIUM`. They print artifact directories; downloaded live data is not committed.
