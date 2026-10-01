import test from "node:test";
import assert from "node:assert/strict";
import { ComparisonController } from "./controller.ts";
import type { Rpc } from "../portfolio/api.ts";
const input = {
  subjects: [
    { text: "AAPL", exchange: null },
    { text: "MSFT", exchange: null },
  ],
  period_end: "2024-12-31",
  years: 3,
  revenue_basis: "contract_revenue_excluding_tax",
  question: null,
  facts_instance: null,
  resolution_instance: null,
  previous_id: null,
} as const;
const record = (id: string) => ({
  id,
  package_id: "p",
  request: { ...input, request_id: "original" },
  companies: [],
  row_count: 0,
  source_count: 0,
  state: "complete",
  created_at: "2026-09-30T00:00:00Z",
  issues: [],
  previous_id: null,
});
const empty = { items: [], next_offset: null };
test("submission retry preserves identity; refresh has new identity and retains saved result on failure", async () => {
  const sent: Record<string, unknown>[] = [];
  let fail = true;
  const rpc: Rpc = async <T>(raw: object) => {
    const c = (raw as { command: Record<string, unknown> }).command;
    if (c.kind === "providers") return { facts: [], resolution: [] } as T;
    if (c.kind === "read") return record(String(c.id)) as T;
    if (c.kind === "start") {
      sent.push(c.request as Record<string, unknown>);
      if (fail) {
        fail = false;
        throw Error("Connection lost");
      }
      return {
        id: "job",
        state: "failed",
        comparison_id: null,
        error: { message: "Source unavailable" },
      } as T;
    }
    return empty as T;
  };
  const c = new ComparisonController(rpc);
  await c.open("conversation", "AAPL");
  await c.select("saved");
  await c.create(input);
  assert.match(c.getSnapshot().error, /Connection lost/);
  await c.create(input);
  assert.equal(sent[0].request_id, sent[1].request_id);
  assert.equal(c.getSnapshot().selected?.id, "saved");
  await c.refresh(c.getSnapshot().selected!);
  assert.notEqual(sent[2].request_id, sent[1].request_id);
  assert.equal(sent[2].previous_id, "saved");
  c.dispose();
});
test("opening saved research only reads and stale responses cannot replace another company", async () => {
  let release!: (value: unknown) => void;
  const calls: string[] = [];
  const rpc: Rpc = async <T>(raw: object) => {
    const c = (raw as { command: Record<string, unknown> }).command;
    calls.push(String(c.kind));
    if (c.kind === "providers") return { facts: [], resolution: [] } as T;
    if (c.kind === "read" && c.id === "old")
      return (await new Promise<unknown>(
        (resolve) => (release = resolve),
      )) as T;
    if (c.kind === "read") return record("new") as T;
    return empty as T;
  };
  const c = new ComparisonController(rpc);
  await c.open("a", "AAPL");
  const old = c.select("old");
  await c.open("b", "MSFT");
  await c.select("new");
  release(record("old"));
  await old;
  assert.equal(c.getSnapshot().selected?.id, "new");
  assert(!calls.includes("start"));
  c.dispose();
});
function pendingJob() {
  return { id: "active", state: "running", comparison_id: null, error: null };
}
function commands(raw: object) {
  return (raw as { command: Record<string, unknown> }).command;
}
async function tick() {
  await new Promise((resolve) => setTimeout(resolve, 250));
}
test("a lost refresh response reuses its request until a receipt arrives", async () => {
  const sent: Record<string, unknown>[] = [];
  let lost = true;
  const rpc: Rpc = async <T>(raw: object) => {
    const cmd = commands(raw);
    if (cmd.kind === "start") {
      sent.push(cmd.request as Record<string, unknown>);
      if (lost) throw Error("Response lost");
      return { ...pendingJob(), state: "cancelled" } as T;
    }
    if (cmd.kind === "providers") return { facts: [], resolution: [] } as T;
    return empty as T;
  };
  const c = new ComparisonController(rpc);
  try {
    await c.open("a", "AAPL");
    await c.refresh(record("saved"));
    lost = false;
    await c.refresh(record("saved"));
    assert.equal(sent[0].request_id, sent[1].request_id);
    await c.refresh(record("saved"));
    assert.notEqual(sent[1].request_id, sent[2].request_id);
  } finally {
    c.dispose();
  }
});
test("status failures retain progress and cancellation, then reconnect", async () => {
  let statuses = 0,
    cancelled = false;
  const rpc: Rpc = async <T>(raw: object) => {
    const cmd = commands(raw);
    if (cmd.kind === "start") return pendingJob() as T;
    if (cmd.kind === "status") {
      statuses++;
      if (statuses === 1) throw Error("Temporary disconnect");
      return {
        ...pendingJob(),
        state: cancelled ? "cancelled" : "running",
      } as T;
    }
    if (cmd.kind === "cancel") {
      cancelled = true;
      return {} as T;
    }
    if (cmd.kind === "providers") return { facts: [], resolution: [] } as T;
    return empty as T;
  };
  const c = new ComparisonController(rpc);
  try {
    await c.open("a", "AAPL");
    await c.create(input);
    await tick();
    assert.equal(c.getSnapshot().busy, true);
    await new Promise((r) => setTimeout(r, 1100));
    assert(statuses >= 2);
    await c.cancel();
    assert.equal(c.getSnapshot().job?.state, "cancelled");
    assert.equal(c.getSnapshot().busy, false);
  } finally {
    c.dispose();
  }
});
test("reopening a company resumes its active job after visiting another company", async () => {
  let statuses = 0;
  const rpc: Rpc = async <T>(raw: object) => {
    const cmd = commands(raw);
    if (cmd.kind === "start") return pendingJob() as T;
    if (cmd.kind === "status") {
      statuses++;
      assert.equal(cmd.conversation, "a");
      return pendingJob() as T;
    }
    if (cmd.kind === "providers") return { facts: [], resolution: [] } as T;
    return empty as T;
  };
  const c = new ComparisonController(rpc);
  try {
    await c.open("a", "AAPL");
    await c.create(input);
    await c.open("a", "AAPL");
    assert.equal(c.getSnapshot().job?.id, "active");
    assert.equal(c.getSnapshot().busy, true);
    await c.open("b", "MSFT");
    assert.equal(c.getSnapshot().job, null);
    await c.open("a", "AAPL");
    await tick();
    assert.equal(c.getSnapshot().job?.id, "active");
    assert(statuses > 0);
  } finally {
    c.dispose();
  }
});
test("a delayed admission receipt remains discoverable after switching companies", async () => {
  let release!: (value: unknown) => void;
  const rpc: Rpc = async <T>(raw: object) => {
    const cmd = commands(raw);
    if (cmd.kind === "start")
      return (await new Promise<unknown>((r) => (release = r))) as T;
    if (cmd.kind === "status") return pendingJob() as T;
    if (cmd.kind === "providers") return { facts: [], resolution: [] } as T;
    return empty as T;
  };
  const c = new ComparisonController(rpc);
  try {
    await c.open("a", "AAPL");
    const start = c.create(input);
    await c.open("b", "MSFT");
    release(pendingJob());
    await start;
    assert.equal(c.getSnapshot().job, null);
    await c.open("a", "AAPL");
    assert.equal(c.getSnapshot().job?.id, "active");
  } finally {
    c.dispose();
  }
});
