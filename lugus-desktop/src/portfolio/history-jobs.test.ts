import { test } from "node:test";
import assert from "node:assert/strict";
import { createHistoryJobs } from "./history-jobs.ts";
import type { PortfolioApi } from "./api.ts";
import type { PortfolioHistoryHeader } from "./types.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
function header(id: string, portfolioId: string, status = "running") {
  return {
    id,
    key: { portfolio_id: portfolioId },
    status,
  } as PortfolioHistoryHeader;
}

test("hiding while history start is pending cancels that job and drains before reuse", async () => {
  const pending = deferred<PortfolioHistoryHeader>();
  const calls: { kind: string; id?: string; portfolio_id?: string }[] = [];
  const api = (async (command: {
    kind: string;
    id?: string;
    portfolio_id?: string;
  }) => {
    calls.push(command);
    if (command.kind === "history_start") return pending.promise;
    if (command.kind === "history_status")
      return header(command.id!, command.portfolio_id!, "cancelled");
    return null;
  }) as PortfolioApi;
  const jobs = createHistoryJobs(api);
  const start = jobs.start({ portfolio_id: "old" });
  const cleanup = jobs.cancel();
  let drained = false;
  const nextSelection = jobs.cancel().then(() => {
    drained = true;
  });
  await Promise.resolve();
  assert.equal(drained, false);
  pending.resolve(header("late-job", "old"));
  await Promise.all([start, cleanup, nextSelection]);
  assert.equal(drained, true);
  assert.deepEqual(
    calls.filter((c) => c.kind === "history_cancel"),
    [{ kind: "history_cancel", portfolio_id: "old", id: "late-job" }],
  );
});

test("late cleanup and release cannot replace ownership of the next job", async () => {
  const old = deferred<PortfolioHistoryHeader>();
  const fresh = deferred<PortfolioHistoryHeader>();
  const cancelled: string[] = [];
  let starts = 0;
  const api = (async (command: {
    kind: string;
    id?: string;
    portfolio_id?: string;
  }) => {
    if (command.kind === "history_start")
      return ++starts === 1 ? old.promise : fresh.promise;
    if (command.kind === "history_cancel") cancelled.push(command.id!);
    if (command.kind === "history_status")
      return header(command.id!, command.portfolio_id!, "cancelled");
    return null;
  }) as PortfolioApi;
  const jobs = createHistoryJobs(api);
  const first = jobs.start({ portfolio_id: "old" });
  const cleanup = jobs.cancel();
  const second = jobs.start({ portfolio_id: "new" });
  old.resolve(header("old-job", "old"));
  await cleanup;
  jobs.release(first);
  assert.deepEqual(cancelled, ["old-job"]);
  const nextCleanup = jobs.cancel();
  fresh.resolve(header("new-job", "new"));
  await Promise.all([second, nextCleanup]);
  assert.deepEqual(cancelled, ["old-job", "new-job"]);
});

test("finished starts require no cancellation", async () => {
  const calls: string[] = [];
  const api = (async (command: { kind: string }) => {
    calls.push(command.kind);
    return header("finished", "portfolio", "complete");
  }) as PortfolioApi;
  const jobs = createHistoryJobs(api);
  const result = jobs.start({});
  await result;
  jobs.release(result);
  await jobs.cancel();
  assert.deepEqual(calls, ["history_start"]);
});

test("a cancellation failure does not poison later selection retries", async () => {
  let cancellations = 0;
  const api = (async (command: {
    kind: string;
    id?: string;
    portfolio_id?: string;
  }) => {
    if (command.kind === "history_start") return header("job", "portfolio");
    if (command.kind === "history_cancel" && ++cancellations === 1)
      throw new Error("Temporary transport failure");
    if (command.kind === "history_status")
      return header(command.id!, command.portfolio_id!, "cancelled");
    return null;
  }) as PortfolioApi;
  const jobs = createHistoryJobs(api);
  await jobs.start({});
  await assert.rejects(jobs.cancel(), /Temporary transport failure/);
  await jobs.cancel();
  await jobs.start({});
  await jobs.cancel();
  assert.equal(cancellations, 2);
});
