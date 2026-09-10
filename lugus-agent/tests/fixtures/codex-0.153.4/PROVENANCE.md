# Codex app-server protocol fixture provenance

- Codex executable: `/Users/havismat/.local/bin/codex`
- Version: `codex-cli 0.153.4`
- Generation command: `/Users/havismat/.local/bin/codex app-server generate-json-schema --experimental --out /tmp/lugus-codex-01534-schema`

This directory deliberately contains only the schemas used by the Codex adapter:
initialize, thread start, turn start and interrupt, dynamic tool call, account read,
and turn completion. Each copied schema retains its local `definitions`, making its
`#/definitions/...` references self-contained. It does not vendor the complete
app-server schema bundle.
