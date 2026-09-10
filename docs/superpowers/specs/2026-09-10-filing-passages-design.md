# HTML filing text and passage references

Date: 2026-09-10. Status: approved by the user; HTML-only scope confirmed.

## Scope and implementation base

Implement milestone 4 against `codex/durable-conversations`, currently at
`42ff3ae` in `/private/tmp/lugus-backend-development`. The main checkout does not
contain the completed application runtime. Preserve pending documents in main;
merging, pushing and publishing remain separate actions.

The user approved immutable text representations, source mappings, exact passage
references and frozen conversation context, then requested reconsidering PDF
support because SEC filings provide HTML. This design narrows initial extraction
to HTML, including Inline XBRL. PDF extraction, OCR, XML/SGML extraction, desktop
rendering and durable investment-review passage capture are deferred. Original
document retrieval continues to preserve bytes for every supported provider format.

SEC guidance describes HTML and ASCII submissions and format-specific exceptions;
Inline XBRL embeds financial tags into HTML. HTML support therefore serves the
initial research workflow without claiming coverage of all EDGAR submissions.

Sources:
- https://www.sec.gov/submit-filings/filer-support-resources/how-do-i-guides/observe-data-process-filing-limits
- https://www.sec.gov/submit-filings/filer-support-resources/edgar-glossary

## Architectural choice

Use a local, replaceable HTML extraction adapter and application-owned immutable
representations. Keep pure extraction-policy, range-validation and passage-building
functions separate from repository reads, SQLite writes, clocks and IDs.

Provider-side extraction would tie passage interpretation to provider releases and
duplicate work across providers. Browser-driven extraction would add a rendering
runtime and environment-dependent text behavior before the desktop milestone.
Local structural extraction provides reproducible offline text with explicit
limitations. It does not claim browser layout fidelity.

Original document bytes and observations stay in `lugus-financial`. Its bounded
`EvidenceRepository::document` adapter already verifies the saved association and
checksum. New representation and passage storage belongs in `lugus-app`, adjacent
to the scoped dataset and conversation references that consume it. Do not add an
extraction capability to the SEC provider or change financial observation identity.

## Immutable representation contract

A representation retains:

- Application identity, workspace, repository and exact document dataset ID.
- Original `DocumentObservation`, including provider, URL, media type, retrieval
  time and original byte checksum.
- Format, extraction-policy version, parser/decoder implementation version and
  effective decoding choice.
- Canonical UTF-8 text checksum, byte length and immutable text chunks.
- Ordered mappings between canonical text ranges and source locations, plus
  explicit extraction limitations.

The application generates representation IDs. A checksum establishes integrity,
not authorization. Scoped dataset reads establish access. Identical bytes from a
different workspace do not grant access to an existing representation.

An explicit prepare operation reads the original saved document, checks bounds,
extracts and validates text, then atomically persists a complete representation.
The cache key includes the scoped dataset and complete extractor identity. An
identical preparation returns the original representation; a new extractor version
creates a new representation. Never overwrite or silently recompute old text.

Reads require an explicit representation ID and bounded range. Reading text,
passages or conversation history performs no extraction or network operation.

## HTML policy and source mapping

Parse HTML with a maintained parser rather than stripping tags with regular
expressions. Pin parser behavior through the lockfile and recorded extractor
identity. Define deterministic decoding, entity handling, whitespace normalization
and structural separators in a versioned extraction policy. Unsupported encodings
or decoding failures produce an explicit error rather than silent character loss.

Preserve headings, paragraph boundaries and table row/cell separation. Retain
visible Inline XBRL fact text without converting numeric values, signs or units.
Exclude script, style, head, template and Inline XBRL hidden metadata subtrees.
Honor HTML hidden attributes and explicitly supported inline hiding declarations.
Do not execute scripts, fetch resources, evaluate external stylesheets, reconstruct
financial statements or claim computed CSS visibility. Include that limitation in
representation metadata.

Source locations identify text nodes in the deterministic parsed source tree,
using structural paths and UTF-8 ranges within entity-decoded node text. These
coordinates are explicitly distinct from raw HTML byte offsets, browser UTF-16
offsets and screen coordinates. Store the mapped source-node text needed to resolve
locations offline. Node paths are pinned to this representation's parser version.

Mapping segments associate canonical output ranges with source-node ranges.
Collapsed whitespace retains its contributing source range; inserted structural
separators are explicitly synthetic. Never claim a one-to-one offset mapping after
normalization. A source-resolution read returns the exact document observation,
mapped node locations and bounded source text; it never searches the latest URL
for matching words. Browser highlighting is a later transport/renderer concern.

## Passage contract

Create a passage from an exact representation ID, a half-open UTF-8 byte range
`[start, end)` and the caller's expected selected text. Validate ordering, nonempty
selection, checked arithmetic, Unicode boundaries, configured bounds and exact
quote equality. Reject selections containing only synthetic separators.

The stored passage contains its own ID, scope, representation identity, document
provenance, extraction identity, full-text checksum, selected range, exact quote,
quote checksum and intersecting source mappings. Derive those fields from trusted
stored content; clients cannot supply replacement provenance or source coordinates.

Creation uses transactional request deduplication: identical input under the same
workspace/request returns the original passage; changed input conflicts. Validate
the full bounded stored/result envelope before committing. Manual and agent output
envelopes retain their existing separate limits and durable-result semantics.

A source-resolution operation accepts a scoped passage ID and returns its pinned
locations. It must still work after a newer same-URL document is ingested, after
another extraction is prepared, after the original tab closes and after restart.

## Storage, effects and bounds

Add an application schema v4 migration preserving existing datasets, bindings,
conversation inputs and review storage. Persist bounded chunks and mappings so a
small read does not deserialize a whole filing. Use indexed ordinal/range reads
with size preflight before allocation and one consistent SQLite snapshot per read.

Validate finite limits for original bytes, parsed nodes/depth, extracted bytes,
mapping count/bytes, text pages, passage bytes and source-resolution responses.
Check limits during parsing/emission as well as before persistence. No partially
successful extraction, truncated quote or silently omitted required context.

CPU extraction runs outside the async executor and outside the application-store
mutex. Load bounded owned input under the store boundary, release it, extract,
then acquire it again for deduplicated atomic persistence. Bound extraction
concurrency and cooperative work; cancellation must not publish a partial record
or leave unbounded background work. No transaction spans extraction or a network
await. A failed preparation leaves the original document readable.

Use safe existing error categories for unsupported media/encoding, invalid ranges,
stale or unavailable evidence, conflicting requests, resource limits and storage
failures. Surface no raw parser diagnostics or source HTML in error messages.

## Application and conversation integration

Expose shared scoped manual APIs and strict agent tools for preparing text,
reading text pages, creating/reading passages and resolving source locations.
Agents can request these local operations without routine confirmation. Model
arguments never supply workspace/run identity or authoritatively supplied evidence.

Extend `SelectedReference` with `Passage { id }`. Extend frozen-reference
construction and restoration validation together. Admission reads the passage
through scoped trusted storage and freezes its bounded quote, provenance and
mappings into the existing untrusted context-data envelope. Do not promote filing
text into system instructions. Selected dataset references retain their existing
metadata semantics; they do not implicitly become extracted documents.

Required passage context must fit existing selected-reference and context limits
before admitting a turn. Existing request deduplication returns the original
frozen context without rereading newer evidence. Old conversation snapshots remain
readable and are never rewritten. Document any additive context payload versioning.

The CLI exposes the same operations and a deterministic actual `AgentRuntime`
acceptance flow. No desktop UI or live model is required to establish the contract.

## Acceptance and regression coverage

1. A realistic HTML/Inline XBRL fixture includes entities, Unicode, nested inline
   tags, hidden metadata, headings and a table. Assert expected readable text and
   source mappings independently of implementation output.
2. Fetch/store the original fixture through the existing document flow, prepare
   text, create a passage and ask about it in a real conversation turn. Capture the
   actual runtime request and prove exact selected text/provenance are present.
3. Resolve that passage to the original document and source-node text. Verify
   cross-node selections and normalization mappings, not just quote equality.
4. Ingest changed bytes at the same URL, prepare the new text, shut down, remove
   provider availability and reopen offline. The original passage, source mappings
   and frozen conversation remain byte-for-byte unchanged. A fresh passage pins
   the newer representation; old offsets are never interpreted against it.
5. Cover invalid UTF-8 boundaries, stale expected quotes, duplicate text, synthetic
   separators, malformed HTML, unsupported formats/encoding, size/depth limits,
   escaping overhead, forged scope, concurrent deduplication and failed persistence.
6. Verify schema migration preserves prior records and old conversation context;
   rerun workspace tests, strict Clippy, formatting and relevant adapter suites.

Update the roadmap's milestone 4 scope and exit criteria to HTML explicitly when
implementing this approved refinement. Record PDF/OCR as deferred work so milestone
completion cannot imply that PDF extraction was validated.
