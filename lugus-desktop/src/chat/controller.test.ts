import { test } from "node:test";
import assert from "node:assert/strict";
import { ChatController } from "./controller.ts";
const chat = (id: string) => ({
  id,
  title: id,
  workspace_id: id,
  created_at: "",
});
const page = { items: [], next_offset: null };
test("late history cannot overwrite another conversation and drafts follow selection", async () => {
  let resolve!: (value: any) => void;
  const pending = new Promise((r) => (resolve = r));
  const controller = new ChatController(async (request: any) =>
    request.operation === "messages" && request.conversation === "a"
      ? pending
      : request.operation === "workspace"
        ? { revision: 0, view_ids: [], selected_view_id: null }
        : (page as any),
  );
  controller.setDraft("new draft");
  controller.setHint("NEW");
  const first = controller.openChat(chat("a"));
  controller.setDraft("A draft");
  controller.setHint("A");
  await controller.openChat(chat("b"));
  controller.setDraft("B draft");
  resolve({
    items: [{ id: "late", text: "wrong conversation" }],
    next_offset: null,
  });
  await first;
  assert.equal(controller.getSnapshot().selection.id, "b");
  assert.deepEqual(controller.getSnapshot().messages, []);
  controller.newChat();
  assert.equal(controller.getSnapshot().draft, "new draft");
  assert.equal(controller.getSnapshot().hint, "NEW");
  await controller.openChat(chat("a"));
  assert.equal(controller.getSnapshot().draft, "A draft");
  assert.equal(controller.getSnapshot().hint, "A");
  controller.dispose();
});
test("failed sends reuse request identity and a later edit survives successful submission", async () => {
  const sent: any[] = [];
  let resolve!: (value: any) => void;
  const controller = new ChatController(async (request: any) => {
    if (request.operation === "send") {
      sent.push(request);
      if (sent.length === 1) throw new Error("connection lost");
      return await new Promise((r) => (resolve = r));
    }
    if (request.operation === "workspace")
      return { revision: 0, view_ids: [], selected_view_id: null } as any;
    return page as any;
  });
  controller.applySettings({
    selected: "codex",
    offline: false,
    runtime_available: true,
    agents: [],
  });
  await controller.openChat(chat("a"));
  controller.setDraft("Question");
  await controller.send();
  const retry = controller.send();
  controller.setDraft("Next question");
  resolve({
    id: "run",
    conversation_id: "a",
    status: "completed",
    error: null,
  });
  await retry;
  assert.equal(sent[0].request, sent[1].request);
  assert.equal(controller.getSnapshot().draft, "Next question");
  controller.dispose();
});
test("late workspace response cannot replace a newer saved-view revision", async () => {
  let resolve!: (value: any) => void;
  let reads = 0;
  const controller = new ChatController(async (request: any) => {
    if (request.operation === "workspace") {
      reads++;
      if (reads === 2) return await new Promise((r) => (resolve = r));
      return { revision: reads, view_ids: [], selected_view_id: null } as any;
    }
    return page as any;
  });
  await controller.openChat(chat("a"));
  const origin = controller.getSnapshot().selection;
  const stale = controller.loadResearch(origin, true);
  await controller.loadResearch(origin, true);
  resolve({ revision: 2, view_ids: [], selected_view_id: null });
  await stale;
  assert.equal(controller.getSnapshot().workspace?.revision, 3);
  controller.dispose();
});
test("saved chats open offline and changed agent settings do not mutate the current run", async () => {
  const requests: string[] = [];
  const controller = new ChatController(async (request: any) => {
    requests.push(request.operation);
    if (request.operation === "runs")
      return {
        items: [
          { id: "old", conversation_id: "a", status: "completed", error: null },
        ],
        next_offset: null,
      } as any;
    if (request.operation === "workspace")
      return { revision: 0, view_ids: [], selected_view_id: null } as any;
    return page as any;
  });
  controller.applySettings({
    selected: "codex",
    offline: true,
    runtime_available: false,
    agents: [],
  });
  await controller.openChat(chat("a"));
  controller.setDraft("Saved draft");
  await controller.send();
  assert.equal(requests.includes("send"), false);
  controller.applySettings({
    selected: "claude_code",
    offline: false,
    runtime_available: true,
    agents: [],
  });
  assert.equal(controller.getSnapshot().run?.id, "old");
  assert.equal(controller.getSnapshot().draft, "Saved draft");
  controller.dispose();
});
test("a failed send cannot display its error in a different selected conversation", async () => {
  let reject!: (error: unknown) => void;
  const controller = new ChatController(async (request: any) => {
    if (request.operation === "send")
      return await new Promise((_r, j) => (reject = j));
    if (request.operation === "workspace")
      return { revision: 0, view_ids: [], selected_view_id: null } as any;
    return page as any;
  });
  controller.applySettings({
    selected: "codex",
    offline: false,
    runtime_available: true,
    agents: [],
  });
  await controller.openChat(chat("a"));
  controller.setDraft("Question");
  const sending = controller.send();
  await controller.openChat(chat("b"));
  controller.setDraft("Keep this draft");
  reject(new Error("Old failure"));
  await sending;
  assert.equal(controller.getSnapshot().error, "");
  assert.equal(controller.getSnapshot().draft, "Keep this draft");
  controller.dispose();
});
test("holding research prepares a scoped snapshot and draft without sending", async () => {
  const requests: any[] = [];
  const snapshots: any[] = [];
  const controller = new ChatController(async (request: any) => {
    requests.push(request);
    if (request.operation === "workspace")
      return {
        revision: 4,
        view_ids: [],
        selected_view_id: "saved-view",
      } as any;
    if (request.operation === "send")
      return {
        id: "run",
        conversation_id: "a",
        status: "completed",
        error: null,
      } as any;
    return page as any;
  });
  controller.applySettings({
    selected: "codex",
    offline: false,
    runtime_available: true,
    agents: [],
  });
  await controller.openChat(chat("a"));
  controller.setDraft("Keep my question.");
  await controller.usePortfolio(
    async (command: object) => {
      snapshots.push(command);
      return { id: "snapshot", summary: { as_of: "2026-09-15" } } as any;
    },
    { id: "portfolio", revision: 7 } as any,
    "account",
    { name: "Example company", symbol: "TEST" },
  );
  assert.equal(
    requests.some((request) => request.operation === "send"),
    false,
  );
  assert.equal(snapshots[0].request.account_id, "account");
  assert.equal(snapshots[0].request.expected_revision, 7);
  assert.equal(snapshots[0].request.conversation_id, "a");
  assert.match(controller.getSnapshot().draft, /Keep my question\./);
  assert.match(
    controller.getSnapshot().draft,
    /Review Example company \(TEST\)/,
  );
  assert.equal(controller.getSnapshot().hint, "TEST");
  await controller.send();
  const submission = requests.find((request) => request.operation === "send");
  assert.deepEqual(submission.selected, [
    { kind: "view", id: "saved-view" },
    { kind: "portfolio", id: "snapshot" },
  ]);
  controller.dispose();
});
test("late snapshot completion is rejected when conversation selection changes", async () => {
  let resolve!: (value: any) => void;
  const controller = new ChatController(async (request: any) =>
    request.operation === "workspace"
      ? { revision: 0, view_ids: [], selected_view_id: null }
      : (page as any),
  );
  await controller.openChat(chat("a"));
  const sharing = controller.usePortfolio(
    async () => await new Promise((r) => (resolve = r)),
    { id: "portfolio", revision: 7 } as any,
    null,
  );
  await controller.openChat(chat("b"));
  resolve({ id: "snapshot", summary: { as_of: "2026-09-15" } });
  await assert.rejects(sharing, /selected conversation changed/);
  assert.equal(controller.getSnapshot().portfolioContext, null);
  controller.dispose();
});
test("plain bridge errors keep their readable message", () => {
  const controller = new ChatController(async () => page as any);
  controller.setError({ kind: "transport", message: "Agent is unavailable" });
  assert.equal(controller.getSnapshot().error, "Agent is unavailable");
  controller.dispose();
});
test("sharing a portfolio from a new chat preserves the unsent draft without an intent", async () => {
  const controller = new ChatController(async (request: any) =>
    request.operation === "create"
      ? chat("created")
      : request.operation === "workspace"
        ? { revision: 0, view_ids: [], selected_view_id: null }
        : (page as any),
  );
  controller.setDraft("My existing question");
  await controller.usePortfolio(
    async () => ({ id: "snapshot", summary: { as_of: "2026-09-15" } }) as any,
    { id: "portfolio", revision: 1 } as any,
    null,
  );
  assert.equal(controller.getSnapshot().draft, "My existing question");
  controller.dispose();
});
test("active saved runs stream progress, keep identity through agent changes, and stop explicitly", async () => {
  let resolve!: (value: any) => void;
  const requests: any[] = [];
  const run = {
    id: "active",
    conversation_id: "a",
    status: "running",
    error: null,
  };
  const controller = new ChatController(async (request: any) => {
    requests.push(request);
    if (request.operation === "runs")
      return { items: [run], next_offset: null } as any;
    if (request.operation === "status")
      return await new Promise((r) => (resolve = r));
    if (request.operation === "activity")
      return {
        items: [
          {
            kind: "runtime",
            data: JSON.stringify({
              type: "text_delta",
              text: "Evidence found",
            }),
          },
          {
            kind: "runtime",
            data: JSON.stringify({
              type: "tool_started",
              name: "fetch_prices",
            }),
          },
        ],
        next_offset: null,
      } as any;
    if (request.operation === "workspace")
      return { revision: 0, view_ids: [], selected_view_id: null } as any;
    if (request.operation === "cancel")
      return { ...run, status: "interrupted" } as any;
    return page as any;
  });
  await controller.openChat(chat("a"));
  controller.applySettings({
    selected: "claude_code",
    offline: false,
    runtime_available: true,
    agents: [],
  });
  assert.equal(controller.getSnapshot().run?.id, "active");
  resolve(run);
  await new Promise((r) => setImmediate(r));
  assert.equal(controller.getSnapshot().stream, "Evidence found");
  assert.equal(controller.getSnapshot().activity, "Retrieving source data…");
  await controller.stop();
  assert.equal(controller.getSnapshot().run?.status, "interrupted");
  assert.equal(
    requests.find((request) => request.operation === "cancel").run,
    "active",
  );
  assert.equal(
    requests.some((request) => request.operation === "send"),
    false,
  );
  controller.dispose();
});

test("company context follows selection and review retries preserve intent", async () => {
  const sent: any[] = [];
  const controller = new ChatController(async (request: any) => {
    if (request.operation === "company") {
      sent.push(request.command);
      if (sent.length === 1) throw new Error("connection lost");
      return { id: "r", conversation_id: "a", status: "completed", error: null } as any;
    }
    if (request.operation === "workspace") return { revision: 0, view_ids: [], selected_view_id: null } as any;
    return page as any;
  });
  controller.applySettings({ selected: "codex", offline: false, runtime_available: true, agents: [] });
  await controller.openCompany({ id: "company-a", name: "Apple", hint: "AAPL", conversation_id: "a", revision: 2, updated_at: "" });
  assert.equal(controller.getSnapshot().company?.id, "company-a");
  assert.equal(controller.getSnapshot().hint, "AAPL");
  controller.prepareReview();
  const draft = controller.getSnapshot().draft;
  assert.match(draft, /thesis/i);
  await controller.send();
  await controller.send();
  assert.equal(sent[0].review, true);
  assert.equal(sent[0].request, sent[1].request);
  assert.equal(sent[0].company, "company-a");
  controller.setDraft("Apple draft");
  await controller.openChat(chat("b"));
  assert.equal(controller.getSnapshot().company, null);
  assert.equal(controller.getSnapshot().review, false);
  await controller.openCompany({ id: "company-a", name: "Apple", hint: "AAPL", conversation_id: "a", revision: 2, updated_at: "" });
  assert.equal(controller.getSnapshot().draft, "Apple draft");
  controller.dispose();
});


test("saved conversation history continues beyond 100 pages", async () => {
  const controller = new ChatController(async (request: any) => {
    if (request.operation === "messages") {
      return {items:[{id:String(request.offset),text:"saved message"}],next_offset:request.offset<100?request.offset+1:null} as any;
    }
    if (request.operation === "workspace")
      return {revision:0,view_ids:[],selected_view_id:null} as any;
    return page as any;
  });
  await controller.openChat(chat("large"));
  assert.equal(controller.getSnapshot().messages.length,101);
  controller.dispose();
});
