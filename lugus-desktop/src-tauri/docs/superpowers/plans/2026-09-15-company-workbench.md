# Company workbench implementation plan

**Goal:** Deliver the approved persistent company research workbench and context-aware chat.
**Architecture:** Desktop SQLite company records associate one enduring conversation with each company. An optional application request field freezes the brief separately from message text. Existing evidence tools and research preparation serve the company conversation.
**Tech stack:** Rust, SQLite, Tauri, React, TypeScript, existing shadcn components.
**Spec:** ../specs/2026-09-15-company-workbench.md

## Constraints

Preserve the existing uncommitted UI redesign. No automatic retrieval on selection. User controls the thesis and acceptance of findings. No new network dependencies. Bounds and stale revisions fail explicitly. Keep old command behavior compatible.

## Tasks

- [x] 1. Add native company command tests in `tests/companies.rs`: create, edit, conflicting revisions, reopen, conversation isolation, invalid source messages. Run `cargo test --test companies` to establish failures.
- [x] 2. Implement `src/companies.rs` and bridge dispatch. Use `rusqlite` matching the existing 0.39 bundled dependency. Persist company records, revisions, accepted findings and frozen sends transactionally. `Company` returns `id`, `name`, `hint`, `conversation_id`, `revision`, `thesis`, `questions`, `findings`, timestamps; commands list/create/get/save/finding/history/send. Validate originating messages through ConversationHost before persisting findings. Page lists at 100 records.
- [x] 3. Extend `SendMessageRequest` with `#[serde(default, skip_serializing_if = "Option::is_none")] pub research_brief: Option<String>`. Bound at 24 KiB; reserve serialized space before context selection and insert into the prompt JSON envelope. Update Rust construction sites with `None`. Verify a deterministic runtime sees the brief and user messages retain their original text; replay must freeze the same brief.
- [x] 4. Add company routing tests to `src/chat/controller.test.ts`; implement optional company context in ChatController. Company sends route through the company bridge operation and include review intent in retry identity. Selection clears context before loading another conversation. Keep per-conversation drafts and explicit portfolio snapshots.
- [x] 5. Add `src/companies/types.ts`, `api.ts`, `workbench.tsx`. Provide company list/create, brief editor, accepted findings, brief revision history and reviews. Wire `src/app.tsx` navigation, context chip, save-finding action, and company conversation selection. Use existing components and responsive CSS. Preserve unsaved edits in component state keyed by company.
- [x] 6. Verify `npm test`, `npm run build`, native tests, relevant application tests and browser flow. Inspect diff for user changes and document limits in the desktop README. No commits of unrelated existing changes.


## Verification results

- Frontend: 49 tests pass; production build passes.
- Desktop: all 23 native tests pass, including 5 company-specific unit/integration checks.
- Application: full lugus-app test suite passes.
- Browser: company workbench, existing redesign/navigation, portfolio accounting/snapshot, and research-workspace flows pass against local QA fixtures.
- Independent review: resolved brief byte budget, originating-run evidence provenance, interrupted association reconciliation, and history pagination/previous successful review lookup.
- Existing UI redesign changes preserved; no commits or publication performed. Live model behavior and native macOS packaging were not exercised.
