# Company workbench

Approved direction: Companies organize ongoing research; opening a company supplies its saved context to the agent. Investors explicitly save findings and edit their own thesis. Portfolio remains available across companies.

## First release

Companies have a user-entered name and resolution hint (not verified identity), an editable thesis and open questions, accepted findings with originating message and evidence references, immutable brief revisions, and a dedicated persistent research conversation. Company selection opens saved material without retrieval. A visible context indicator explains that the saved brief accompanies messages. The agent treats the thesis as a hypothesis, checks counterevidence, and proposes changes without editing it.

An explicit Review thesis action prepares a question asking for fresh evidence and comparison with the saved thesis and previous review. It does not send automatically. Review turns are identified durably and expose their status and completed answer in history. Failed or interrupted reviews never masquerade as completed assessments. All company messages include a frozen, bounded research brief separate from user-visible message text. Replays retain the original brief even after edits. Existing conversation evidence remains available through the current scoped tools.

## Persistence and boundaries

The desktop owns a SQLite company store adjacent to the application database. It records company/conversation associations, optimistic revisions, findings, and frozen send requests. Application conversation admission adds an optional research_brief string to the frozen prompt envelope with explicit size bounds. Existing callers and stored requests without the field retain their existing serialization. Source access remains scoped to the company's persistent conversation; opening a company does not imply verified source identity. Portfolio sharing stays explicit through existing snapshots.

The UI uses existing React/shadcn components and responsive navigation. Company switching preserves chat drafts and unsaved brief edits. Save failures remain visible and do not discard edits. Existing unrelated conversations remain accessible. No background monitoring, valuation modeling, automatic thesis rewriting, broker import, or full document reader is included.

## Verification

Native tests cover persistence/reopen, stale edits, frozen request replay, company scoping, invalid finding origins, review states, and brief delivery without altering user text. Frontend tests cover switching/drafts, company routing and review submission. Browser verification covers add/select/edit/reopen, save a finding, and responsive company navigation. Existing frontend/native suites and builds must pass.
