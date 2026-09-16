#!/usr/bin/env python3
"""Synthetic Claude CLI: records native search permissions, never accesses a model."""
import json
import sys
import time
from pathlib import Path

if "--version" in sys.argv:
    print("2.1.268 (Claude Code)")
    sys.exit(0)

instructions = sys.argv[sys.argv.index("--system-prompt") + 1]
payload = json.load(sys.stdin)
latest = json.loads(payload["prompt"])["new_message"]["text"]
interpretation = instructions.startswith("Interpret the latest user request")
builtins = sys.argv[sys.argv.index("--tools") + 1]
allowed = sys.argv[sys.argv.index("--allowedTools") + 1]
with Path("search-sessions.jsonl").open("a") as log:
    log.write(json.dumps({"interpretation": interpretation, "builtins": builtins, "allowed": allowed}) + "\n")

if interpretation:
    assert builtins == ""
    Path("interpreting").touch()
    deadline = time.monotonic() + 10
    while Path("pause").exists():
        assert time.monotonic() < deadline, "test did not release interpretation"
        time.sleep(0.01)
    result = json.dumps({
        "workflow": "conversation" if "ordinary" in latest else "web_search",
        "subjects": [], "start": None, "end": None, "clarification": None,
    })
else:
    result = "Fixture answer with [source](https://example.test/news)."
print(json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": result}), flush=True)
