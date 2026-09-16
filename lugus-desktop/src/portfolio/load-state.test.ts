import { test } from "node:test";
import assert from "node:assert/strict";
import {
  beginPortfolioLoad,
  failPortfolioLoad,
  finishPortfolioLoad,
  finishPortfolioRefresh,
  portfolioScope,
  selectedPortfolioLoad,
} from "./load-state.ts";
import type { View } from "./types.ts";

const firstScope = portfolioScope("portfolio", "first-account");
const secondScope = portfolioScope("portfolio", "second-account");
const firstView = {
  id: "portfolio",
  accounts: [{ id: "first-account" }],
} as View;

test("a failed account selection cannot display or share the prior account view", () => {
  const loaded = finishPortfolioLoad(
    beginPortfolioLoad(firstScope),
    firstScope,
    [],
    firstView,
    firstView,
  );
  const immediate = selectedPortfolioLoad(loaded, secondScope);
  assert.equal(immediate.status, "loading");
  assert.equal(immediate.view, null);
  assert.equal(immediate.whole, null);
  const failed = failPortfolioLoad(
    beginPortfolioLoad(secondScope),
    secondScope,
    "Account unavailable",
  );
  assert.equal(failed.status, "error");
  assert.equal(failed.view, null);
  assert.equal(failed.whole, null);
  assert.equal(
    finishPortfolioLoad(failed, firstScope, [], firstView, firstView),
    failed,
  );
});

test("an initial list error remains distinguishable from a successful empty portfolio list", () => {
  const scope = portfolioScope(null, null);
  const failed = failPortfolioLoad(
    beginPortfolioLoad(scope),
    scope,
    "Storage unavailable",
  );
  assert.equal(failed.status, "error");
  assert.equal(failed.error, "Storage unavailable");
  const retry = beginPortfolioLoad(scope);
  assert.equal(retry.status, "loading");
  const empty = finishPortfolioLoad(retry, scope, [], null, null);
  assert.equal(empty.status, "ready");
  assert.deepEqual(empty.headers, []);
  assert.equal(empty.error, "");
});

test("a slow history refresh cannot overwrite a newer save in the same account scope", async () => {
  const beforeSave = { ...firstView, revision: "4", name: "Before save" };
  const afterSave = { ...firstView, revision: "5", name: "Saved name" };
  let state = finishPortfolioLoad(
    beginPortfolioLoad(firstScope),
    firstScope,
    [],
    beforeSave,
    beforeSave,
  );
  let finishHistory!: () => void;
  const history = new Promise<void>((resolve) => {
    finishHistory = resolve;
  });
  const refresh = history.then(() => {
    state = finishPortfolioRefresh(
      state,
      firstScope,
      "4",
      beforeSave,
      beforeSave,
    );
  });
  state = beginPortfolioLoad(firstScope);
  state = finishPortfolioLoad(state, firstScope, [], afterSave, afterSave);
  finishHistory();
  await refresh;
  assert.equal(state.view?.revision, "5");
  assert.equal(state.whole?.name, "Saved name");
});

test("a refresh that completes during a same-scope reload cannot end the loading state", () => {
  const captured = { ...firstView, revision: "4" };
  const reloading = beginPortfolioLoad(firstScope);
  assert.equal(
    finishPortfolioRefresh(reloading, firstScope, "4", captured, captured),
    reloading,
  );
  const loaded = finishPortfolioLoad(
    reloading,
    firstScope,
    [],
    captured,
    captured,
  );
  const repriced = { ...captured, name: "New price snapshot" };
  assert.equal(
    finishPortfolioRefresh(loaded, firstScope, "4", repriced, repriced).view,
    repriced,
  );
});
