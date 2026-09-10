#!/usr/bin/env python3

import json
import os
import sys
import time

legacy_scenarios = {
    "oversized", "partial", "malformed", "invalid_utf8", "silent",
    "slow_trickle", "blocked_stdin", "eof", "stderr_flood", "ignore_eof",
    "stderr_descendant", "echo",
}
scenario = sys.argv[1] if len(sys.argv) > 1 and sys.argv[1] in legacy_scenarios else os.path.basename(sys.argv[0])

if len(sys.argv) > 1 and sys.argv[1] == "--version":
    if scenario == "bad_version":
        print("codex-cli 0.154.0")
    elif scenario == "oversized_version":
        print("x" * 5000)
    else:
        print("codex-cli 0.153.4")
    sys.exit(0)


def read_message():
    line = sys.stdin.readline()
    if not line:
        sys.exit(0)
    return json.loads(line)


def send(message):
    print(json.dumps(message, separators=(",", ":")), flush=True)


def rpc_result(request, result):
    send({"id": request["id"], "result": result})


def thread(index):
    return {
        "cliVersion": "0.153.4", "createdAt": 1, "cwd": os.getcwd(),
        "ephemeral": True, "id": f"thread-{index}", "modelProvider": "test-provider",
        "preview": "", "projectId": None, "sessionId": f"session-{index}",
        "source": "appServer", "status": "idle", "turns": [], "updatedAt": 1,
    }


def turn(turn_id, status="inProgress", items=None):
    return {"id": turn_id, "items": items or [], "status": status}


def validate_client_request(message, expected_id, method):
    if message.get("id") != expected_id or message.get("method") != method:
        send({"id": message.get("id", expected_id), "error": {"code": -32600, "message": "unexpected client request"}})
        sys.exit(1)


def serve_session():
    expected_startup_args = [
        "-c", "features.shell_tool=false",
        "-c", "features.hooks=false",
        "-c", "features.apps=false",
        "-c", "features.plugins=false",
        "-c", "features.browser_use=false",
        "-c", "features.computer_use=false",
        "-c", "features.image_generation=false",
        "-c", "features.view_image=false",
        "-c", "features.multi_agent=false",
        "-c", "features.goals=false",
        "-c", "features.sleep_tool=false",
        "-c", "features.tool_suggest=false",
        "-c", "features.skill_search=false",
        "-c", "features.request_permissions_tool=false",
        "-c", "tools.experimental_request_user_input.enabled=false",
        "-c", "tools.update_plan.enabled=false",
        "-c", "skills.include_instructions=false",
        "-c", "skills.bundled.enabled=false",
        "-c", "project_doc_max_bytes=0",
        "-c", 'developer_instructions=""',
        "-c", 'web_search="live"',
        "-c", 'approval_policy="never"',
        "-c", 'sandbox_mode="read-only"',
    ]
    if sys.argv[2:] != expected_startup_args:
        sys.exit(1)
    initialize = read_message()
    expected_initialize = {
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {"name": "lugus", "version": "0.1.0"},
            "capabilities": {"experimentalApi": True},
        },
    }
    if initialize != expected_initialize:
        send({"id": initialize.get("id", 1), "error": {"code": -32600, "message": "initialize required"}})
        return
    rpc_result(initialize, {
        "codexHome": os.getcwd(), "platformFamily": "unix",
        "platformOs": sys.platform, "userAgent": "codex-cli/0.153.4",
    })
    if read_message() != {"method": "initialized"}:
        return

    next_id = 2
    thread_index = 0
    config_reads = 0
    while True:
        message = read_message()
        if message.get("method") == "account/read":
            validate_client_request(message, next_id, "account/read")
            next_id += 1
            if message.get("params") != {}:
                sys.exit(1)
            if scenario == "login_required":
                rpc_result(message, {"account": None, "requiresOpenaiAuth": True})
            elif scenario == "provider_auth":
                rpc_result(message, {"account": None, "requiresOpenaiAuth": False})
            else:
                rpc_result(message, {"account": {"type": "apiKey"}, "requiresOpenaiAuth": True})
            continue

        if message.get("method") == "config/read":
            validate_client_request(message, next_id, "config/read")
            next_id += 1
            config_reads += 1
            if message.get("params") != {"cwd": os.getcwd(), "includeLayers": False}:
                sys.exit(1)
            config = {"mcp_servers": {"inherited": {"command": "unused"}}}
            if scenario == "malformed_config":
                config = {"mcp_servers": []}
            elif scenario == "refresh_config" and config_reads == 2:
                config = {"mcp_servers": {"newly_inherited": {"command": "unused"}}}
            rpc_result(message, {"config": config, "origins": {}})
            continue

        validate_client_request(message, next_id, "thread/start")
        next_id += 1
        if scenario == "rpc_error":
            send({"id": message["id"], "error": {"code": -32000, "message": "thread rejected"}})
            continue

        thread_index += 1
        thread_id = f"thread-{thread_index}"
        params = message.get("params", {})
        tools = params.get("dynamicTools")
        expected_tool = {
            "type": "function", "name": "lugus_recall",
            "description": "Recall stored findings",
            "inputSchema": {
                "type": "object",
                "properties": {"thesis_id": {"type": "string"}},
                "required": ["thesis_id"], "additionalProperties": False,
            },
        }
        expected_mcp_servers = {"inherited": {"enabled": False}}
        if scenario == "refresh_config" and thread_index == 2:
            expected_mcp_servers = {"newly_inherited": {"enabled": False}}
        expected_config = {
            "features": {"shell_tool": False, "hooks": False, "apps": False, "plugins": False,
                         "browser_use": False, "computer_use": False, "image_generation": False,
                         "view_image": False, "multi_agent": False, "goals": False, "sleep_tool": False,
                         "tool_suggest": False, "skill_search": False, "request_permissions_tool": False},
            "tools": {"experimental_request_user_input": {"enabled": False}, "update_plan": {"enabled": False}},
            "skills": {"include_instructions": False, "bundled": {"enabled": False}},
            "project_doc_max_bytes": 0, "developer_instructions": "", "web_search": "live",
            "mcp_servers": expected_mcp_servers,
        }
        if params.get("cwd") != os.getcwd() or params.get("baseInstructions") != "You are the Lugus review agent." or params.get("developerInstructions") != "Use only the supplied thesis context." or tools != [expected_tool] or params.get("approvalPolicy") != "never" or params.get("sandbox") != "read-only" or params.get("config") != expected_config:
            send({"id": message["id"], "error": {"code": -32602, "message": "invalid thread settings"}})
            continue
        if scenario == "normal" and (params.get("model") != "gpt-test" or params.get("modelProvider") != "test-provider"):
            send({"id": message["id"], "error": {"code": -32602, "message": "missing model overrides"}})
            continue
        rpc_result(message, {
            "activePermissionProfile": None, "approvalPolicy": "never", "approvalsReviewer": "disabled",
            "cwd": os.getcwd(), "instructionSources": [], "model": params.get("model") or "gpt-test",
            "modelProvider": params.get("modelProvider") or "test-provider", "multiAgentMode": "explicitRequestOnly",
            "reasoningEffort": None, "runtimeWorkspaceRoots": [], "sandbox": {"type": "readOnly"},
            "serviceTier": None, "thread": thread(thread_index),
        })

        start = read_message()
        validate_client_request(start, next_id, "turn/start")
        next_id += 1
        turn_id = f"turn-{thread_index}"
        if start.get("params") != {"threadId": thread_id, "input": [{"type": "text", "text": "Review thesis A"}]}:
            send({"id": start["id"], "error": {"code": -32602, "message": "invalid turn settings"}})
            continue
        rpc_result(start, {"turn": turn(turn_id)})

        if scenario == "interruptible":
            interrupt = read_message()
            if interrupt.get("id") != next_id or interrupt.get("method") != "turn/interrupt" or interrupt.get("params") != {"threadId": thread_id, "turnId": turn_id}:
                sys.exit(1)
            next_id += 1
            rpc_result(interrupt, {})
            send({"method": "turn/completed", "params": {"threadId": thread_id, "turn": turn(turn_id, "interrupted", [])}})
            continue

        if scenario == "approval":
            send({"id": "approval-1", "method": "item/commandExecution/requestApproval", "params": {}})
            if read_message() != {"id": "approval-1", "result": {"decision": "cancel"}}:
                sys.exit(1)
            continue

        if scenario == "human_input":
            send({"id": "input-1", "method": "item/tool/requestUserInput", "params": {}})
            response = read_message()
            if response.get("id") != "input-1" or response.get("error", {}).get("code") != -32601:
                sys.exit(1)
            continue

        if scenario == "process_death":
            sys.exit(0)

        tool_name = "missing_tool" if scenario == "unknown_tool" else "lugus_recall"
        namespace = "foreign" if scenario == "namespaced_tool" else None
        call_thread = "thread-other" if scenario == "wrong_thread" else thread_id
        send({
            "id": f"server-{thread_index}", "method": "item/tool/call",
            "params": {"arguments": {"thesis_id": "thesis-A"},
                       "callId": "call-1", "threadId": call_thread, "tool": tool_name,
                       "turnId": turn_id, "namespace": namespace},
        })
        if scenario == "wrong_thread":
            continue
        tool_response = read_message()
        success = scenario not in {"unknown_tool", "namespaced_tool", "oversized_result"}
        if scenario == "unknown_tool":
            expected_content = "unknown tool: missing_tool"
        elif scenario == "namespaced_tool":
            expected_content = "unknown tool: foreign/lugus_recall"
        elif scenario == "oversized_result":
            expected_content = "tool "
        else:
            expected_content = "Stored finding from thesis A"
        expected_response = {
            "id": f"server-{thread_index}",
            "result": {"contentItems": [{"type": "inputText", "text": expected_content}], "success": success},
        }
        if tool_response != expected_response:
            sys.exit(1)

        if scenario == "tool_limit":
            send({
                "id": f"server-limit-{thread_index}", "method": "item/tool/call",
                "params": {"arguments": {"thesis_id": "thesis-A"}, "callId": "call-2",
                           "threadId": thread_id, "tool": "lugus_recall", "turnId": turn_id},
            })
            response = read_message()
            if response.get("id") != f"server-limit-{thread_index}" or response.get("result", {}).get("success") is not False:
                sys.exit(1)
            continue

        if scenario == "multiple_messages":
            send({"method": "item/agentMessage/delta", "params": {
                "delta": "Draft response.", "itemId": "message-draft", "threadId": thread_id, "turnId": turn_id}})
            send({"method": "item/agentMessage/delta", "params": {
                "delta": "Final ", "itemId": "message-final", "threadId": thread_id, "turnId": turn_id}})
            send({"method": "item/agentMessage/delta", "params": {
                "delta": "response.", "itemId": "message-final", "threadId": thread_id, "turnId": turn_id}})
            send({"method": "turn/completed", "params": {"threadId": thread_id, "turn": turn(turn_id, "completed", [])}})
        else:
            send({"method": "item/agentMessage/delta", "params": {
                "delta": "Assessment uses ", "itemId": "message-1", "threadId": thread_id, "turnId": turn_id}})
            send({"method": "item/agentMessage/delta", "params": {
                "delta": "stored finding from thesis A.", "itemId": "message-1", "threadId": thread_id, "turnId": turn_id}})
            item = {"id": "message-1", "text": "Assessment uses stored finding from thesis A.", "type": "agentMessage"}
            send({"method": "item/completed", "params": {"item": item, "threadId": thread_id, "turnId": turn_id}})
            send({"method": "turn/completed", "params": {"threadId": thread_id, "turn": turn(turn_id, "completed", [item])}})


if len(sys.argv) > 1 and sys.argv[1] == "app-server":
    serve_session()
    sys.exit(0)

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
elif scenario == "slow_trickle":
    for byte in b'{"id": 1}\n':
        sys.stdout.buffer.write(bytes([byte]))
        sys.stdout.buffer.flush()
        time.sleep(0.1)
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
elif scenario == "stderr_descendant":
    import subprocess

    subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(2)"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=sys.stderr,
    )
    print(json.dumps({"ready": True}), flush=True)
elif scenario == "echo":
    for line in sys.stdin:
        message = json.loads(line)
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
