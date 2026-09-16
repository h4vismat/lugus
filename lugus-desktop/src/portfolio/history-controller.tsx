import { useEffect, useMemo, useRef, useState } from "react";
import type { PortfolioApi } from "./api";
import type {
  View,
  PortfolioHistoryHeader,
  PortfolioHistoryPage,
  PerformancePoint,
  HistoryEvidencePage,
} from "./types";
import { historyRange, type Period } from "./history-state";
import { errorText } from "./forms";
import { createHistoryJobs } from "./history-jobs";
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
export function usePortfolioHistory(
  api: PortfolioApi,
  view: View | null,
  accountId: string | null,
  online: boolean,
  visible: boolean,
) {
  const [period, setPeriod] = useState<Period>("YTD");
  const [header, setHeader] = useState<PortfolioHistoryHeader | null>(null);
  const [rows, setRows] = useState<PerformancePoint[]>([]);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const selected = useRef(view);
  selected.current = view;
  const jobs = useMemo(() => createHistoryJobs(api), [api]);
  const cancel = () => jobs.cancel();
  function validate(
    page: PortfolioHistoryPage | HistoryEvidencePage,
    h: PortfolioHistoryHeader,
  ) {
    if (
      page.result_id !== h.id ||
      JSON.stringify(page.key) !== JSON.stringify(h.key)
    )
      throw new Error("Saved history page does not match the selected result.");
  }
  async function load(force = false, updatedView?: View) {
    const v = updatedView ?? selected.current;
    if (!v || !visible) return;
    const origin = ++generation.current;
    const active = () => origin === generation.current;
    async function show(h: PortfolioHistoryHeader) {
      const points: PerformancePoint[] = [];
      let offset = 0;
      do {
        const p = await api<PortfolioHistoryPage>({
          kind: "history_read",
          portfolio_id: v!.id,
          id: h.id,
          offset,
        });
        if (!active()) return;
        validate(p, h);
        for (const point of p.items) {
          if (points.length && point.date <= points.at(-1)!.date)
            throw new Error("Historical dates did not advance.");
          points.push(point);
        }
        if (p.next_offset === null) break;
        if (p.next_offset !== points.length || p.next_offset <= offset)
          throw new Error("History pagination did not advance.");
        offset = p.next_offset;
      } while (true);
      if (points.length !== h.row_count)
        throw new Error("Saved historical data is incomplete.");
      if (!active()) return;
      setHeader(h);
      setRows(points);
      setStatus(
        `${h.key.revision !== v!.revision ? "Saved result · " : ""}${h.status.replaceAll("_", " ")} · ${h.baseline ?? "Unknown baseline"} → ${h.effective_end ?? "Unknown end"}${h.issue_count ? ` · ${h.issue_count} coverage notes; see daily data` : ""}${h.evidence_count ? " · Source completeness unverified" : ""}${h.issues.some((i) => i.code === "benchmark_selection_required") ? " · Choose a benchmark source in Accounts." : ""}${h.error ? ` · ${h.error}` : ""}`,
      );
    }
    try {
      await cancel();
      if (!active()) return;
      setHeader(null);
      setRows([]);
      setBusy(true);
      setStatus("Loading historical performance…");
      const day = new Intl.DateTimeFormat("en-CA", {
        timeZone: "America/New_York",
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
      }).format(new Date());
      const range = historyRange(period, v.as_of < day ? v.as_of : day);
      const cached = await api<PortfolioHistoryHeader | null>({
        kind: "history_latest",
        portfolio_id: v.id,
        account_id: accountId,
        range,
      });
      if (!active()) return;
      if (cached) await show(cached);
      if (!active()) return;
      if (!online) {
        if (!cached) setStatus("Offline · No saved history for this period.");
        return;
      }
      if (cached) setStatus("Refreshing saved history…");
      const pending = jobs.start({
        request_id: crypto.randomUUID(),
        portfolio_id: v.id,
        account_id: accountId,
        expected_revision: v.revision,
        range,
        refresh: force ? "force" : "missing",
      });
      let h = await pending;
      if (!active()) return;
      while (h.status === "running") {
        await wait(350);
        if (!active()) return;
        h = await api<PortfolioHistoryHeader>({
          kind: "history_status",
          portfolio_id: v.id,
          id: h.id,
        });
      }
      if (!active()) return;
      jobs.release(pending);
      if (h.status === "complete" || h.status === "partial") await show(h);
      else
        setStatus(
          `${h.status.replaceAll("_", " ")}${h.error ? `: ${h.error}` : ""}${cached ? " · Saved history remains available." : ""}`,
        );
    } catch (e) {
      if (active()) setStatus(errorText(e));
    } finally {
      if (active()) setBusy(false);
    }
  }
  useEffect(() => {
    if (visible && view) void load();
    return () => {
      generation.current++;
      void cancel().catch(() => {});
    };
  }, [api, view?.id, view?.revision, accountId, online, visible, period]);
  async function dataPage(offset: number) {
    if (!header) throw new Error("No saved history selected.");
    const p = await api<PortfolioHistoryPage>({
      kind: "history_read",
      portfolio_id: header.key.portfolio_id,
      id: header.id,
      offset,
    });
    validate(p, header);
    return p;
  }
  async function evidencePage(offset: number) {
    if (!header) throw new Error("No saved history selected.");
    const p = await api<HistoryEvidencePage>({
      kind: "history_evidence",
      portfolio_id: header.key.portfolio_id,
      id: header.id,
      offset,
    });
    validate(p, header);
    return p;
  }
  return {
    period,
    setPeriod,
    header,
    rows,
    status,
    busy,
    online,
    refresh: (updatedView?: View) => load(true, updatedView),
    cancel: async () => {
      try {
        await cancel();
      } catch (e) {
        setStatus(errorText(e));
      }
    },
    dataPage,
    evidencePage,
  };
}
export type HistoryController = ReturnType<typeof usePortfolioHistory>;
