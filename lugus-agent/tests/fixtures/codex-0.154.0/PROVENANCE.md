# Codex 0.154.0 compatibility verification

Generated locally using `codex-cli 0.154.0`:

```sh
codex app-server generate-json-schema --experimental --out /tmp/lugus-codex-01540-schema
```

Compared all thirteen schemas used by the adapter against the existing 0.153.4 fixtures, ignoring descriptive text. All request, notification and dynamic-tool schemas are structurally identical. ThreadStartResponse only adds optional Thread fields `daybreakEnabled`, `environments`, and `originator`, plus the ThreadEnvironment definition. The changed response schema is retained here; the unchanged contracts remain in ../codex-0.153.4. Existing response parsing tolerates these additions.

The adapter explicitly accepts 0.153.4 and 0.154.0. A session/tool-call regression exercises the 0.154.0 version probe; unknown versions remain rejected.
