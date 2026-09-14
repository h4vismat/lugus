# Lugus

Lugus is a financial research application that combines agent-assisted chat, company research, historical price charts, and reported financial data. A native macOS desktop app sits on top of a Rust backend, with conversations and retrieved evidence stored locally in SQLite.

> **Alpha — not ready for production use.** Lugus is under active development. Expect updates and breaking changes to features, APIs, configuration, and stored-data formats. Backward compatibility is not guaranteed during the alpha.

## What it does

- Tracks USD stock, ETF and cash portfolios from transactions or opening lots, with FIFO P&L and explicit price refresh.
- Accepts research requests in chat, with an optional company or ticker hint.
- Resolves company identities and retrieves evidence through configured data providers.
- Displays saved historical price charts and reported financial metrics alongside conversations.
- Preserves source identifiers, reporting periods, units, retrieval history, and missing or conflicting observations.
- Reopens saved conversations and research offline without automatically refreshing financial data.
- Supports Codex and Claude Code through separate agent runtime adapters.

Agent configuration and financial data providers are separate: an authenticated agent alone does not supply company data. The bundled provider plugins cover SEC EDGAR filings and fundamentals, and daily market data through yfinance.

## Architecture

| Component | Responsibility |
| --- | --- |
| [lugus-desktop](lugus-desktop/README.md) | Tauri desktop shell and TypeScript research interface. |
| [lugus-app](lugus-app/README.md) | Framework-independent application host, research preparation, durable conversations, evidence, and views. |
| [lugus-portfolio](lugus-portfolio/README.md) | Exact decimal accounting, FIFO lots, transaction replay and valuation. |
| [lugus-agent](lugus-agent/README.md) | Agent runtime adapters, tool execution boundaries, and investment review workflows. |
| [lugus-financial](lugus-financial/README.md) | Financial domain types, provider capabilities, ingestion, provenance, and SQLite persistence. |

The backend separates pure domain validation and selection functions from process execution, storage, and application coordination. Agent runtimes use provider-neutral interfaces; financial plugins run as external processes over a versioned JSON-RPC protocol. Vendor SDKs stay outside the financial core. Adding a provider still requires implementing the relevant capabilities and any necessary identifier or binding adapters.

In desktop research, application code validates the interpreted request and retrieves evidence before analysis. The analysis agent reads the saved evidence through scoped tools. Local storage does not imply local model inference: the configured agent CLI may send prompts, context, and tool results to its model provider.

## Getting started

### Build the macOS app

Prerequisites:

- macOS 13 or newer and Xcode Command Line Tools.
- Rust with Cargo and support for the Rust 2024 edition.
- Node.js 22.12 or newer and npm.

From the repository root:

```sh
cd lugus-desktop
npm ci
npm run tauri -- build --bundles app
open src-tauri/target/release/bundle/macos/Lugus.app
```

The bundle path assumes the default Cargo target directory. The desktop is a separate Cargo workspace and is not built by root workspace commands.

### Configure agents and data providers

On first launch, Lugus creates configuration and local storage under its macOS application-data directory, normally `~/Library/Application Support/dev.lugus.desktop/`. The initial configuration has no financial data providers.

Follow the [desktop configuration guide](lugus-desktop/README.md#local-configuration) to configure an authenticated agent CLI and a dedicated runtime directory outside any Git repository. Select the agent in **Settings**. The supported CLI versions and runtime options are documented in that guide.

Install and configure the providers needed for your research:

- [SEC EDGAR](lugus-financial/plugins/sec-edgar/README.md): company resolution, filings, and fundamentals. Requires its Python environment and your own application/contact identity.
- [yfinance](lugus-financial/plugins/yfinance/README.md): historical daily market data. Requires its separate Python environment and pinned dependencies.

See [provider configuration](lugus-desktop/README.md#configure-real-data-providers) for manifest entries and installation commands. Agent selection is available in Settings; executable paths, models, and provider configuration currently use local JSON files.

To open existing research without agent or provider execution, follow the [offline instructions](lugus-desktop/README.md#open-saved-research-offline).

## Development

From the repository root, check the Rust backend:

```sh
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Rust integration tests require `python3` on `PATH` for subprocess fixtures. Component guides describe additional plugin tests and optional live checks.

Check the desktop separately:

```sh
cd lugus-desktop
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --tests
```

The [application guide](lugus-app/README.md#deterministic-cli-acceptance) includes a synthetic CLI workflow for exercising the backend without model credentials or live financial sources. The [desktop test guide](lugus-desktop/tests/README.md) describes native QA fixtures.

## Current limitations

Lugus is an evolving alpha research surface. Retrieval depends on explicitly configured provider capabilities and source coverage. Charts show historical observations, not live quotes. Reported metrics preserve source periods and units; they do not constitute reconciled financial statements or derived quarterly values.

There is no automatic background data refresh, full filing-document renderer, or complete provider/model configuration UI. See the [desktop guide](lugus-desktop/README.md#current-limits) for further limitations and the component documentation for detailed contracts.
