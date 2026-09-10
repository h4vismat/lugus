## Status

DONE

## What I implemented

- Added `lugus-agent/examples/codex_session.rs`, a runnable current-thread Tokio example accepting `WORKSPACE`, `PROMPT`, and optional `--model`, `--provider`, `--codex`, and `--cancel-after-ms` arguments.
- Bound the read-only `lugus_context` demonstration tool with the exact required fixed result and an empty-object input schema.
- Streamed provider-neutral runtime events to stdout, printed completed/cancelled outcomes, checked Codex-owned authentication, and closed the disposable runtime on success or error.
- Added `lugus-agent/README.md` with Codex 0.153.4 setup, Codex-owned authentication, dedicated-workspace guidance, provider/model configuration, remote-provider disclosure, deterministic cancellation, verification commands, runtime/event boundaries, and milestone exclusions.
- Documented the controller's event ruling: a host-limit-rejected call emits no tool lifecycle events; an accepted unknown flat call can emit lifecycle events without application-executor invocation; interruption can leave `ToolStarted` unpaired, so the terminal result is authoritative.

## TDD evidence

RED:

```text
$ cargo test -p lugus-agent --example codex_session
error[E0432]: unresolved import `super::parse_args`
error: could not compile `lugus-agent` (example "codex_session" test) due to 1 previous error
exit_code=101
```

This was the expected failure before the CLI parser existed. The tests protect positional/optional argument mapping and rejection of a malformed cancellation delay.

GREEN:

```text
$ cargo test -p lugus-agent --example codex_session
running 2 tests
test tests::rejects_an_invalid_cancellation_delay ... ok
test tests::parses_workspace_prompt_and_optional_runtime_overrides ... ok
test result: ok. 2 passed; 0 failed
exit_code=0
```

## Live verification

All live runs used the user's existing authorized Codex login, the configured default model/provider, and separate fresh empty `/tmp` workspaces. No login was started, no user configuration was changed, and no account identity, credential, prompt payload, response payload, or raw configuration was logged by the diagnostic proxy.

### Host tool and event delivery

The prompt required exactly one `lugus_context` call. The observed sequence was:

```text
event started run_id=codex-session-example
event tool_started call_id=<sanitized> name=lugus_context
event tool_finished call_id=<sanitized> success=true
event text_delta <content omitted here>
outcome completed
```

The final response contained the complete required fixed demonstration result and labeled it application-supplied demonstration data. This confirmed initialize/account/config, dynamic-tool callback/response, event delivery, text streaming, and completion against the real Codex app server.

### Native web search

A separate prompt required native search for the official Rust `std::time::Duration` documentation. A temporary pass-through proxy captured only notification method, item type, and action type. It observed:

```text
{"method":"item/started","item_type":"webSearch","action_type":null}
{"method":"item/completed","item_type":"webSearch","action_type":"search"}
```

The completed answer returned `https://doc.rust-lang.org/std/time/struct.Duration.html`. Native web search is therefore available in the selected configuration. This was a transient runtime result; no document or provenance was persisted, and the temporary proxy/log were removed.

### Active-turn cancellation

The cancellation run started its 500 ms timer immediately before `run`. It observed `Started` before `Cancelled`:

```text
event started run_id=codex-session-example
outcome cancelled
exit_code=0
```

This exercised active-turn interruption rather than cancellation before startup, and the runnable interface returned in under four seconds.

The real tool, search, completion, and interruption message shapes required no adapter change or new protocol regression test.

## Deterministic verification

Run once after implementation:

- `cargo test -p lugus-agent`: 56 passed, 0 failed (24 unit + 23 session integration + 9 runtime contract; doc tests also passed).
- `cargo clippy -p lugus-agent --all-targets -- -D warnings`: exit 0, no warnings.
- `cargo fmt -p lugus-agent -- --check`: exit 0.
- `cargo test -p lugus-financial`: 36 passed, 0 failed (plus 0-test library/doc targets).

## Files changed

- `lugus-agent/examples/codex_session.rs`
- `lugus-agent/README.md`
- `.superpowers/sdd/2026-09-09-agent-harness/task-6-report.md`

## Self-review

- Re-read the task brief and shared contracts after implementation.
- Confirmed the example does not persist an assessment or imply memory, scheduling, evidence capture, or durable web results.
- Confirmed configuration remains provider-selectable and the example depends only on provider-neutral public runtime/tool contracts outside construction of the Codex adapter.
- Corrected the README so lifecycle-event language matches all existing adapter branches and cancellation behavior.
- Reviewed only the scoped new files; no production adapter mismatch was observed, so `tests/codex_session.rs` and runtime source were left unchanged.

No unresolved issues or concerns.
