# Lugus desktop

Chat accepts arbitrary messages and an optional company/ticker hint. The application interprets the request, resolves source-backed company identity, retrieves evidence, and then starts offline analysis. Progress distinguishes interpretation, retrieval and analysis. See [research preparation](../docs/application-research-preparation.md) for supported workflows and limits.

The macOS desktop app places recent conversations on the left, agent chat in the center, and saved company charts and reported financials on the right. Conversations, selected views, and retrieved evidence persist in local SQLite databases. Opening saved research does not fetch fresh financial data. New retrievals are driven by an explicit research request; there is no automatic data refresh.

This is a standalone Cargo package with its own workspace in `src-tauri`, using the repository's `lugus-app`, `lugus-agent`, and `lugus-financial` crates through path dependencies. It is not built by the root Cargo workspace commands. Its frontend uses Tauri IPC and must run inside the native app for research operations.

## Build and run on macOS

Prerequisites: Rust with Cargo, Xcode Command Line Tools, and Node.js 22.12 or newer with npm. The native bundle targets macOS 13 or newer. Python and provider dependencies are needed only for the providers you configure.

From the repository root:

```sh
cd lugus-desktop
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --tests
npm run tauri -- build --bundles app
```

The release bundle is normally written to `src-tauri/target/release/bundle/macos/Lugus.app` (a custom Cargo target directory changes this location). Open that app, or launch its executable from this directory to supply configuration explicitly:

```sh
LUGUS_CONFIG=/absolute/path/to/desktop.json \
  ./src-tauri/target/release/bundle/macos/Lugus.app/Contents/MacOS/lugus-desktop
```

For a faster native debug bundle, use `npm run tauri -- build --debug --bundles app`; its executable is under `src-tauri/target/debug/bundle/macos/Lugus.app/Contents/MacOS/`.

## Local configuration

Without `LUGUS_CONFIG`, first launch creates `desktop.json`, `application.json`, and a `runtime` directory in the app's macOS application-data directory, normally `~/Library/Application Support/dev.lugus.desktop/`. The initial application has no data providers. On this first creation only, it looks for the executable specified by `LUGUS_CODEX`, or otherwise `~/.local/bin/codex`; if that file is absent, the generated runtime is null. Existing configuration files are not rewritten on later launches.

You can instead create your own configuration directory outside any Git repository. Create its `runtime` subdirectory before enabling the agent. A `desktop.json` with an agent looks like this; replace the executable path with your installed CLI:

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

The Codex adapter requires **Codex CLI 0.153.4**; the Claude Code adapter requires **Claude Code 2.1.268**. Each adapter checks its supported version before starting a session. It uses the CLI's existing authentication and account/provider configuration. Lugus does not provide a login form. For Codex, a null `model` or `model_provider` leaves that choice to the CLI configuration. For Claude Code, a null `model` uses the restricted session default; set `model` explicitly to override it. `timeout_secs` defaults to 180 and must be between 10 and 600.

Runtime paths and `application_config` resolve relative to `desktop.json`. The runtime workspace must already exist, be a directory, and have no Git repository at that directory or any ancestor. The executable must be an existing file. Changes to executable, workspace, model, and provider configuration require restarting Lugus. Selecting an agent in Settings takes effect on the next message without restarting.

### Select Codex or Claude Code

Open **Settings**, choose **Agent**, then **Save agent**. The choice is saved beside the desktop configuration in `desktop.agents.json` (or `<config-name>.agents.json` for a custom filename). A running message retains its original agent for both request interpretation and answering. Each selected profile's timeout bounds the whole message. Switching agents retains Lugus conversation history and saved evidence.

Lugus discovers the CLIs using `LUGUS_CODEX` / `LUGUS_CLAUDE`, then `~/.local/bin`, `PATH`, `/opt/homebrew/bin`, and `/usr/local/bin`. Discovery uses the existing runtime workspace and checks files only; version and authentication are checked when a session starts. Install the supported CLI and authenticate with that CLI before using it. Lugus does not copy or store credentials. Reopen Lugus after installing a CLI.

The existing `runtime` object remains a Codex profile. For custom installations or separate models/workspaces, add named `agents` profiles; a named Codex profile overrides the legacy profile:

```json
{
  "application_config": "application.json",
  "agents": {
    "codex": {
      "executable": "/absolute/path/to/codex",
      "workspace": "runtime",
      "timeout_secs": 180
    },
    "claude_code": {
      "executable": "/absolute/path/to/claude",
      "workspace": "runtime",
      "model": null,
      "timeout_secs": 180
    }
  }
}
```

Claude Code uses its own provider configuration; omit `model_provider` for that profile. Its Rust adapter exposes only the current run's Lugus tools through a temporary authenticated loopback MCP endpoint. Ordinary inherited hooks, plugins, skills and MCP servers are disabled; organization-managed policy remains authoritative. Native web access is disabled for desktop research runs. See [the agent adapter documentation](../lugus-agent/README.md#run-the-claude-code-example) for protocol and verification details.

A minimal `application.json` is:

```json
{
  "financial_path": "financial.sqlite",
  "application_path": "application.sqlite",
  "providers": []
}
```

Database and provider-manifest paths resolve relative to `application.json`. Financial evidence and application state use separate database files. Preserve both when moving saved research.

## Configure real data providers

Agent configuration and financial-provider configuration are separate. An authenticated agent alone does not supply company data. Add installed provider manifests to the application's `providers` array. For example, a market-data entry uses the repository's yfinance adapter:

```json
{
  "instance_id": "market",
  "manifest": "/absolute/path/to/lugus/lugus-financial/plugins/yfinance/plugin.json",
  "active": true,
  "config": {}
}
```

Install that adapter's Python environment and pinned dependencies first. From the repository root:

```sh
python3 -m venv lugus-financial/plugins/yfinance/.venv
lugus-financial/plugins/yfinance/.venv/bin/python -m pip install \
  -r lugus-financial/plugins/yfinance/requirements.txt
```

See the [yfinance provider documentation](../lugus-financial/plugins/yfinance/README.md) for supported identifiers, price semantics, and source limitations.

For SEC company resolution, filings, and fundamentals, add the [SEC EDGAR provider](../lugus-financial/plugins/sec-edgar/README.md), whose manifest is `lugus-financial/plugins/sec-edgar/plugin.json`. Its provider entry must contain `config.user_agent` with **your own application identity and actual contact information**. Supply that value yourself before activating the provider; no contact identity is generated by Lugus. The SEC adapter requires Python 3.10 or newer and EdgarTools. Create `lugus-financial/plugins/sec-edgar/.venv` and install that plugin's `requirements.txt` into it before activating the provider. Its manifest uses `.venv/bin/python` relative to the plugin directory; an installed manifest can instead name an absolute interpreter path. SEC data does not supply the market-price series, so configure the capabilities needed for your research.

Provider processes run from their manifest directories. No provider credentials or contact details belong in this README or in synthetic fixtures.

## Open saved research offline

The runtime is optional. Omit it or set `"runtime": null` to open saved conversations and research without starting an agent or provider process. Alternatively, force offline mode for an existing configuration:

```sh
LUGUS_CONFIG=/absolute/path/to/desktop.json LUGUS_OFFLINE=1 \
  ./src-tauri/target/release/bundle/macos/Lugus.app/Contents/MacOS/lugus-desktop
```

Sending messages requires an enabled runtime. Offline opening can recover previously interrupted run state; it does not replay the turn or refresh its evidence. Window close waits for host shutdown, including active-run cancellation and runtime/provider cleanup.

## Tests and native QA

`npm test` runs the frontend state/data tests. `cargo test --manifest-path src-tauri/Cargo.toml --tests` exercises the real bridge and SQLite persistence with injected deterministic runtimes and providers.

The [test documentation](tests/README.md) describes an explicit synthetic native QA fixture. From `lugus-desktop`, seed a new, nonexistent directory with:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --example seed_chat -- /tmp/lugus-qa-new
```

Launch the native app with that directory's generated `desktop.json`. This fixture contains synthetic prices, facts, and a completed conversation solely for testing. It is not a real financial source or production runtime, and its answers are not linked into the native application. Use a fresh directory for each seed.

## Complete Financials

New financial research opens every SEC metric returned in the saved filing-date range, across namespaces and units. The default shows the latest annual and quarterly originals plus subsequent amendments. Income Statement, Balance Sheet, Cash Flow and Other Metrics have separate controls. Within each section, financials are grouped by exact reporting period, newest first; instant balances and different duration ranges remain separate, and absent metrics appear in a final group. Select All filings or an individual accession to explore the saved history; search and 50-row pagination keep the complete snapshot accessible. The usual research range is five years, not all SEC history.

Metrics absent from the selected filings show Missing. Source details explicitly show absent labels and metadata; exact decimals and original reporting periods remain intact. The desktop loads all saved fact pages (up to 100,000 observations, with an explicit error above that limit). Older selected-metric views remain readable and are labeled partial; request new financial research to create a complete view.

## Current limits

The desktop is an initial research surface, not completion of the wider product roadmap. A full document renderer and an editable provider/model configuration UI are not implemented. Agent selection is editable in Settings; executable, workspace, model, and data-provider configuration remain in local JSON files. Chart windows are bounded to the newest saved rows; displayed prices are historical observations, not live quotes. Reported metrics retain their source periods, units, gaps, and conflicts rather than becoming assembled financial statements. Research depends on the capabilities of explicitly configured providers.

## Simple chart tool

The agent can call `get_price_chart({symbol: "PLTR", start: "2024-01-01", end: "2024-12-31"})`. The backend selects the single eligible active price provider, constructs its native identifier, retrieves bounded paginated daily closes, saves a dataset and accepts a price-chart view. The result includes `fetch_id`, `dataset_id`, `view_id`, row count and source coverage. Dates are inclusive; no automatic refresh is introduced.

The bundled adapter currently supports yfinance 0.2.0 and supplies `yahoo:symbol` internally. The public tool has no provider-specific fields. Multiple eligible providers produce an ambiguity error rather than choosing arbitrarily. A symbol chart does not establish a company binding or join fundamentals by ticker; the explicit binding workflow remains available for company-associated evidence. Failed/cancelled retrieval does not open a successful chart.

Statement categorization uses an offline, replaceable concept map extracted from EdgarTools 5.57.0 (`gaap_mappings.json`, confidence at least 0.8), with its MIT notice under `public/licenses`. Only exact US GAAP concepts are mapped; uncertain and other-taxonomy metrics stay in Other Metrics. These categories do not reproduce the filing presentation or duplicate shared line items across statements. Refresh the map using `python3 scripts/update-statement-mappings.py` after installing the pinned SEC plugin environment.

## Portfolio

Open **Portfolio** in the sidebar, create a portfolio, and add stocks or ETFs under **Accounts**. All accounts and instruments use USD.

- Choose **Full history** to enter deposits, purchases, sales and other activity from the start. Record the funding deposit before a purchase.
- Choose an existing account to enter opening cash and each remaining purchase lot with its original date, remaining quantity and total remaining cost basis including fees. Mark simplified lots or unknown dates explicitly; these assumptions remain visible.
- **Add transaction** supports buys, sells, deposits, withdrawals, dividends, fees and splits. Preview calculations before saving. **Transactions** provides corrections and voids; **Audit** preserves the command history. Split corrections cover the linked accounts together.
- **Overview** shows cash, FIFO realized P&L, unrealized P&L, income and fees. **Holdings** expands to remaining lots. Choose one account or the whole portfolio. Missing prices produce an incomplete subtotal rather than a zero-priced holding.
- Connect an instrument to a configured price source under **Accounts**, then choose **Refresh prices**. The current valuation adapter supports bundled yfinance 0.2.0. Values display observation dates, and older saved observations remain usable offline. Refresh does not update automatically; its cancellation button stops the current job.
- **Use in chat** creates a frozen snapshot and shows a sharing indicator. The next message shares that selection with the configured agent. Without a company hint, the agent analyzes saved portfolio evidence; with a hint, it can combine that snapshot with company research. Later edits require a new snapshot.

Accounting uses decimal strings; amounts display rounded to cents. FIFO is per account. Dividends and standalone fees are shown separately from trading P&L. CSV import, broker synchronization, FX, shorts, margin, tax reports and historical performance charts are outside this version. Desktop list views currently cap at 10,000 rows; select an account to narrow large portfolios. Large snapshot headers or individual rows can exceed configured context/transport budgets and are rejected explicitly.

### Portfolio browser check

The browser test drives the built UI against a real offline native bridge and disposable SQLite databases. It verifies both setup paths, FIFO results, holdings and snapshot selection. It does not call a live market provider or agent.

```sh
npm run build
cargo build --manifest-path src-tauri/Cargo.toml --example portfolio_qa
node tests/portfolio-browser.cjs
```

Install Playwright and its Chromium browser in your test environment first. `LUGUS_PLAYWRIGHT_MODULE` may point to an existing Playwright module; `LUGUS_CHROMIUM` optionally selects a Chromium executable. The test prints the temporary directory containing screenshots and databases. Native bridge tests also run with `cargo test --manifest-path src-tauri/Cargo.toml --tests`.
