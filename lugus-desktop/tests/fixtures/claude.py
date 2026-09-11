#!/usr/bin/env python3
"""Desktop integration peer: synthetic interpretation and answer, no model access."""
import json
import sys
from pathlib import Path

if "--version" in sys.argv:
    print("2.1.268 (Claude Code)")
    sys.exit(0)

instructions = sys.argv[sys.argv.index("--system-prompt") + 1]
payload = json.load(sys.stdin)
assert "Hello" in payload["prompt"]
assert "--restricted" in sys.argv
interpretation = instructions.startswith("Interpret the latest user request")
with Path("sessions").open("a") as log:
    log.write("interpretation\n" if interpretation else "answer\n")
result = json.dumps({
    "workflow": "conversation", "subjects": [], "start": None,
    "end": None, "clarification": None,
}) if interpretation else "Claude fixture answer"
print(json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": result}), flush=True)
