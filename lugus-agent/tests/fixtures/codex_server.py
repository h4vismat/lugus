import json
import sys
import time

scenario = sys.argv[1]

if scenario == "oversized":
    sys.stdout.write("x" * 2048 + "\n")
    sys.stdout.flush()
elif scenario == "partial":
    sys.stdout.write('{"id": 1, ')
    sys.stdout.flush()
    time.sleep(0.2)
    sys.stdout.write('"result": {}}\n')
    sys.stdout.flush()
elif scenario == "malformed":
    print("not-json", flush=True)
elif scenario == "invalid_utf8":
    sys.stdout.buffer.write(b"\xff\n")
    sys.stdout.buffer.flush()
elif scenario == "silent":
    time.sleep(2)
elif scenario == "blocked_stdin":
    time.sleep(2)
elif scenario == "eof":
    pass
elif scenario == "stderr_flood":
    sys.stderr.write("x" * (1024 * 1024))
    sys.stderr.flush()
    for line in sys.stdin:
        message = json.loads(line)
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
elif scenario == "ignore_eof":
    for _line in sys.stdin:
        pass
    while True:
        time.sleep(1)
elif scenario == "echo":
    for line in sys.stdin:
        message = json.loads(line)
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
