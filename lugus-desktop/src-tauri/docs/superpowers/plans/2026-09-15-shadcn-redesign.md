# Lugus shadcn redesign implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development to implement these tasks with independent ownership and review.

**Goal:** Deliver the sage-and-cream redesign across all existing desktop workflows.
**Architecture:** A single React root renders the app shell and feature components. Preserve typed Tauri transport and pure financial helpers; migrate imperative renderers to React components.
**Tech Stack:** React, TypeScript, Vite, Tailwind, shadcn/ui, Tauri.
**Spec:** ../specs/2026-09-15-shadcn-redesign-design.md

## Global constraints
- Preserve existing command payloads, exact decimal transport, stale-selection guards, offline reads, explicit refreshes, preview/save and retry identity.
- All UI is React-owned by completion. No DOM adapter pretending to be a shadcn migration.
- Keep sage, cream and forest-green colors with Georgia display headings.
- Browser fixtures stay in tests. Real native-backed portfolio suites remain the behavioral acceptance gate.

## Interfaces and ownership
- Foundation (root): package.json, lockfile, tsconfig, vite.config.ts, index.html, src/style.css, cn utility package, src/components/ui/*.tsx, tests/redesign-browser.cjs.
- Chat (task 2): src/main.tsx, src/app.tsx, src/chat/*; replaces src/main.ts and owns React shell. Uses ResearchPanel and SettingsDialog interfaces below.
- Research/settings (task 3): src/research/*, src/settings-dialog.tsx; replaces src/research.ts, src/financials-panel.ts, src/settings-dialog.ts. Preserve pure settings.ts, financials.ts, data.ts, types.ts.
- Portfolio (task 4): src/portfolio/*.tsx and related React hooks. Preserve pure helpers and API/types. Entry: PortfolioPanel({api,online,visible,onBack,onUse}) where api is PortfolioApi, online/visible are booleans, onBack is ()=>void, onUse is (view: View,accountId:string|null,intent?:{name:string;symbol:string})=>Promise<void>.
- ResearchPanel({items,receipts,selectedId,issues,onSelect}) with ResearchView[], View[], string|null, string[], (id:string)=>void. Owns tab/range preference state. Export viewLabel(item:ResearchView,all:ResearchView[]):string from src/research/panel.tsx.
- SettingsDialog({open,onOpenChange,rpc,onSaved}) with boolean, (open:boolean)=>void, <T>(request:object)=>Promise<T>, (value:AgentSettings)=>void.

## Task 1: Shared component foundation and responsive theme
- [x] Record current npm test/build baseline; add a browser scenario asserting narrow-window navigation and preserved drafts. Run it against the old UI to demonstrate the missing behavior.
- [x] Install React and Tailwind integration in the existing Vite app; initialize shadcn with editable source using its official CLI.
- [x] Generate Button, Input, Textarea, Label, NativeSelect, Checkbox, Card, Tabs, Badge, Dialog, Sheet, Tooltip, Skeleton, Table components. Add only actual dependencies.
- [x] Define semantic theme variables, focus/disabled states, typography, spacing and responsive workspace layouts. Set index.html to a single #root and /src/main.tsx.
- [x] Run strict build and inspect representative screens after feature integration.

## Task 2: Chat controller and application shell
- [x] Add behavioral coverage for late replies and draft preservation using production controller APIs or browser interaction, and verify failure before implementation.
- [x] Port current main.ts orchestration into a React-compatible controller/hook, preserving polling, pagination, retry IDs, revision and selection guards, selected evidence and portfolio snapshots.
- [x] Build shell/sidebar/navigation, chat empty/loaded/streaming states, message formatting, composer, sidebar sheet, and resizable research area using shared components.
- [x] Integrate ResearchPanel, SettingsDialog and PortfolioPanel with the exact interfaces above. Preserve stable IDs when they describe user controls tested today.
- [x] Verify chat, settings and workspace browser flows and existing pure state tests.

## Task 3: Research and settings React surfaces
- [x] Use existing financial/settings reducer tests as regression baseline; add focused interaction checks for changed behaviors.
- [x] Port research identity, price plots, facts, filings, complete financial statements and source detail into React components with existing pure financial grouping/filtering helpers.
- [x] Use accessible Tabs and shared controls, retain exact-value tables and source limitations. Persist tab/range preferences by selected view.
- [x] Build SettingsDialog around existing reducer with loading, saving, failure and offline states. Prevent dismiss while saving and restore trigger focus.
- [x] Verify type checking, settings tests and financials tests; report integration needs.

## Task 4: Portfolio React surfaces
- [x] Run existing native-backed portfolio browser suites when the integrated app is available. Keep their business assertions throughout migration.
- [x] Port overview/holdings/allocation/performance/history interactions and portfolio state into React components, preserving chart geometry and controller semantics.
- [x] Port account, instrument, binding, transaction, correction, void, preview/save forms and audit UI. Keep exact decimal strings and idempotent retry behavior.
- [x] Use Sheet for holding detail with focus restoration and prepared Research intent. Preserve account scoping, paging, keyboard chart inspection and sources.
- [x] Verify all frontend tests/build and both native-backed portfolio suites; adapt selectors only for deliberate semantic markup changes.

## Task 5: Integration, review and delivery
- [x] Remove superseded imperative rendering modules after confirming no imports remain.
- [x] Run npm test, npm run build, tests/redesign-browser.cjs, tests/portfolio-browser.cjs and tests/portfolio-dashboard-browser.cjs. Use the existing disposable native QA bridge and Playwright installation where available.
- [x] Inspect desktop/narrow screenshots for research, portfolio, settings and dialogs. Fix clipping, contrast, focus and console errors.
- [x] Independently review final implementation against spec and fix material findings; record checks and limitations.
- [x] Update README and design/plan status. Apply the reviewed patch to the user's original checkout with required filesystem approval; do not merge or push without authorization.

Implementation, validation and delivery complete. Applied the reviewed patch to /home/havismat/lugus without creating a merge or changing its branch.
