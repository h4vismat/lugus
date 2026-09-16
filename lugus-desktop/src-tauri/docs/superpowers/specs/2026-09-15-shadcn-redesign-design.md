# Lugus shadcn redesign

Status: implemented and verified on 2026-09-15. User selected refinement of Lugus’s sage-and-cream identity.

## Outcome

Rebuild the desktop application's presentation using React, TypeScript, Tailwind CSS, and editable shadcn/ui components. Deliver a coherent interface across research, chat, portfolio, and settings, with all existing workflows usable.

## Approach and alternatives

Recommended: migrate one complete screen or panel at a time behind a shared React application shell. Verify each migrated surface before continuing; finish this redesign with all current surfaces migrated. This limits the amount of interacting code changed at once while establishing a single component system.

A single full rewrite would simplify the transition structure but make behavioral regressions harder to locate. A permanent mixture of imperative DOM controls and React widgets would reduce initial work but leave two rendering systems to maintain. Temporary legacy mounts may be used during migration, with explicit ownership and cleanup; remove them by completion.

## Visual design

Use warm off-white for the workspace, white cards, pale sage navigation and muted surfaces, forest green primary actions, and restrained terracotta error and negative-value accents. Define semantic CSS variables for background, foreground, card, primary, muted, border, input, ring, destructive, and financial chart/status colors, following shadcn theming conventions.

Keep Georgia for the Lugus wordmark and selected page titles; use the system sans-serif stack for controls and text. Use tabular numerals for financial values. Standardize spacing, radii, control sizes, focus rings, and disabled states. Body text and labels must remain legible in dense financial views. Verify contrast on the actual color combinations.

Use consistent line icons with accessible names on icon-only controls. Use shared Button, Input, Textarea, Label, Select or Native Select, Checkbox, Card, Tabs, Badge, Separator, Dialog, Sheet, Tooltip, Skeleton, and Table components as the screens require. Add only components used by the application.

## Screens

### Application shell

Persistent sidebar with Lugus branding, New chat, Research and Portfolio navigation, recent conversations, connection status, and Settings. Clearly indicate selection. Give the active workspace a consistent header and action placement. On narrow windows, provide a navigation sheet so every destination stays reachable.

### Research and chat

Keep the conversation beside a resizable company-research panel on wide windows. Use a calm empty state with the existing suggested prompts. Group the company hint, selected evidence, message input, and send/stop controls into a clear composer. Show research progress and recoverable errors near the relevant action.

Research exposes saved-view selection and Overview, Financials, and Filings tabs. Preserve source details, period and filing selection, metric search and pagination. At narrow widths, expose research through a labelled toggle and sheet or dedicated panel; retain the conversation draft across navigation.

### Portfolio

Use a page header with portfolio/account selection and clearly ranked actions. Display total, invested value, cash, and returns in a consistent summary; place performance prominently, followed by holdings and allocation. Keep Overview, Holdings, Accounts, Transactions, and Audit reachable.

Use shared table and form controls for account setup, instruments, source bindings, transactions, corrections, and voids. Keep preview and save distinct. Present holding detail in a focus-managed sheet, including lots, activity, and Research. Keep methodology, source links, exact daily data, and accounting details available through labelled disclosures.

### Settings

Use a shared dialog with agent selection, availability details, save feedback, and clear close controls. Keep loading, unavailable, saving, and error states explicit.

## Architecture and data flow

Keep the existing Vite/Tauri packaging. Set the macOS bundle minimum to 13.3 to match Tailwind 4’s Safari/WebKit 16.4 requirement. Add React/React DOM, the React Vite plugin, Tailwind's Vite integration, shadcn configuration and needed component dependencies. Enable strict TSX checking across application source. Pin resolved dependencies in the lockfile.

Organize shared primitives under src/components/ui, application layout under src/components, and feature components/hooks under research, chat, settings, and portfolio folders. Use a single React root. Reuse existing API types, exact-decimal formatting, financial categorization, chart geometry, and pure state helpers where compatible.

Separate transport and state transitions from rendering. React components consume typed data and invoke explicit commands. Give asynchronous work generation/request checks and cleanup so late replies cannot overwrite a different selection. Keep active chat monitoring independent of the currently visible screen. Cancel or dispose portfolio jobs and listeners according to existing lifecycle behavior.

Preserve Tauri command payloads, revision checks, mutation request identity, and retry semantics. Keep domain calculations in existing domain/backend code. React owns DOM within migrated subtrees; legacy code must not modify those nodes.

## Behavioral requirements

- Saved conversations, evidence, selected views, and portfolios continue to open offline.
- Research fetches remain driven by explicit requests; opening saved views does not fetch fresh data.
- Drafts survive workspace navigation. Research from a holding prepares context and text without sending.
- Agent changes apply to subsequent messages; running messages retain their selected agent.
- Portfolio preview, save, retry, correction, and void preserve current revision and idempotency semantics.
- Decimal strings stay exact through transport and form submission. Financial display rounding remains intentional.
- Missing prices, chart gaps, unknown lot dates, incomplete results, and source limitations remain visible.
- Chart ranges do not change current holdings totals. Keyboard inspection, sorting, paging, benchmark methodology, and source links remain available.
- Dialogs and sheets support labelled controls, keyboard use, Escape, appropriate initial focus, and focus restoration.
- Narrow layouts keep navigation and actions reachable; wide tables scroll within their container.

## Validation and completion

Establish a baseline with existing frontend tests and build. Adapt browser selectors to accessible roles where component markup changes while preserving behavioral assertions. Run frontend tests, strict type checking and production build after migration.

Run both deterministic portfolio browser suites against the disposable native bridge and databases. Add focused browser coverage for migrated chat and settings, including draft retention, send/stop, streaming, stale responses, agent selection, failure recovery, and research tab keyboard navigation. Use test fixtures for browser-only states rather than shipping synthetic data in the application.

Inspect screenshots for empty and populated research, portfolio overview, holding detail, forms, and settings at desktop and narrow widths. Verify keyboard navigation and browser console errors. Run native bridge tests if transport integration changes; report any environment limitation rather than claiming unexecuted verification.

The redesign is complete when all existing surfaces use the shared React/shadcn system, legacy rendering is removed, the behavior checks pass, and the inspected screens follow the approved visual direction.

## References

- https://ui.shadcn.com/docs/installation/vite
- https://ui.shadcn.com/docs/theming
- lugus-desktop/README.md
- lugus-desktop/tests/portfolio-browser.cjs
- lugus-desktop/tests/portfolio-dashboard-browser.cjs

## Verification record

- `npm test`: all 48 tests in 10 files pass in the original checkout, including chat selection/retry/streaming and portfolio cancellation/load/refresh regressions.
- `npm run build`: strict TypeScript and production Vite build pass; approximately 452 kB JavaScript (141 kB gzip), 58 kB CSS (12 kB gzip).
- Native-backed portfolio browser suite passes account setup, exact FIFO outcomes and snapshot selection.
- Native-backed dashboard browser suite passes chart controls, keyboard inspection, sorting, focus restoration, narrow layout, preserved drafts, missing-price gaps and offline reopening.
- Research/settings browser suite passes exact source values, filtering, pagination, accessible tab labels, stale loads, save guard, retry and focus restoration.
- Responsive shell browser suite passes navigation, draft/hint retention, research toggling and offline controls.
- Desktop and narrow screenshots inspected. Independent review findings fixed; final review has no blocking findings.
- Packaged macOS launch was not tested in this Linux environment. The compatibility floor is documented and enforced in Tauri configuration.
