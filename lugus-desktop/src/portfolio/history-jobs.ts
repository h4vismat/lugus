import type { PortfolioApi } from "./api.ts";
import type { PortfolioHistoryHeader } from "./types.ts";

interface OwnedJob {
  result: Promise<PortfolioHistoryHeader>;
}

/** Each cancellation owns its captured job, including a start that replies late. */
export function createHistoryJobs(
  api: PortfolioApi,
  wait: (ms: number) => Promise<unknown> = (ms) =>
    new Promise((resolve) => setTimeout(resolve, ms)),
) {
  let current: OwnedJob | null = null;
  let draining: Promise<void> = Promise.resolve();

  function start(request: object) {
    const result = api<PortfolioHistoryHeader>({
      kind: "history_start",
      request,
    });
    current = { result };
    return result;
  }

  function release(result: Promise<PortfolioHistoryHeader>) {
    if (current?.result === result) current = null;
  }

  function cancel(): Promise<void> {
    const previous = current;
    current = null;
    if (!previous) return draining;
    // A later load must also await cleanup already started by an effect teardown.
    const cancellation = draining.then(async () => {
      let header: PortfolioHistoryHeader;
      try {
        header = await previous.result;
      } catch {
        return;
      }
      if (header.status !== "running") return;
      const command = { portfolio_id: header.key.portfolio_id, id: header.id };
      await api({ kind: "history_cancel", ...command });
      for (let i = 0; i < 50; i++) {
        const status = await api<PortfolioHistoryHeader>({
          kind: "history_status",
          ...command,
        });
        if (status.status !== "running") return;
        await wait(100);
      }
    });
    // Report this attempt's failure without making every future retry fail.
    draining = cancellation.catch(() => {});
    return cancellation;
  }

  return { start, release, cancel };
}
