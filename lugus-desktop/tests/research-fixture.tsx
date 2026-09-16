import { useState } from "react";
import { createRoot } from "react-dom/client";
import { ResearchPanel } from "../src/research/panel";
import { SettingsDialog } from "../src/settings-dialog";
import type { AgentSettings } from "../src/settings";
import type { ResearchView, ReportedFinancialFact } from "../src/types";
import "../src/style.css";
const company = { namespace: "sec", value: "123" };
const fact: ReportedFinancialFact = {
  company,
  namespace: "us-gaap",
  concept: "Revenues",
  label: "Revenue",
  unit: "USD",
  value: "9007199254740993.123456789",
  period: { kind: "duration", start: "2025-01-01", end: "2025-12-31" },
  filed: "2026-02-01",
  form: "10-K",
  filing_id: "annual",
  fiscal_year: 2025,
  fiscal_period: "FY",
  source_url: "https://example.test/annual",
  retrieved_at: "2026-09-15T00:00:00Z",
};
const facts = Array.from({ length: 55 }, (_, i) => ({
  ...fact,
  period: {
    kind: "duration",
    start: `${1971 + i}-01-01`,
    end: `${1971 + i}-12-31`,
  },
}));
const item: ResearchView = {
  view: {
    id: "financials",
    dataset_id: "facts",
    kind: "data_table",
    descriptor_revision: 1,
  },
  data: {
    header: {
      id: "facts",
      kind: "facts",
      binding_id: null,
      row_count: facts.length,
      query: { company, filed_from: "2020-01-01", filed_to: "2026-09-15" },
      created_at: "2026-09-15",
      policy: "all-reported-facts-v1",
      provider: {
        instance_id: "fixture",
        plugin_id: "fixture",
        plugin_version: "1",
      },
      limitations: ["Saved source limitation"],
      conflicts: [],
      error: null,
      coverage: null,
    },
    rows: facts.map((value) => ({
      kind: "reported_fact",
      evidence: { value },
    })),
    next_offset: null,
  },
};
const prices: ResearchView = {
  view: {
    id: "prices",
    dataset_id: "prices",
    kind: "price_chart",
    descriptor_revision: 1,
  },
  data: {
    header: {
      ...item.data.header,
      id: "prices",
      kind: "prices",
      row_count: 2,
      policy: null,
    },
    rows: ["2026-08-01", "2026-09-01"].map((date) => ({
      kind: "price",
      value: "123.450001",
      evidence: {
        value: {
          date,
          currency: "USD",
          close: "123.450001",
          retrieved_at: "2026-09-15",
          source_url: "https://example.test/prices",
          instrument: company,
        },
      },
    })),
    next_offset: null,
  },
};
const settings: AgentSettings = {
  selected: "codex",
  offline: false,
  runtime_available: true,
  agents: [
    { id: "codex", label: "Codex", available: true, detail: "Codex ready" },
    {
      id: "claude_code",
      label: "Claude Code",
      available: true,
      detail: "Claude ready",
    },
  ],
};
const queue: {
  request: Record<string, unknown>;
  resolve: (value: AgentSettings) => void;
  reject: (reason: Error) => void;
}[] = [];
Object.assign(window, {
  fixture: {
    queue,
    settings,
    resolve(index: number, value = settings) {
      queue[index].resolve(value);
    },
    reject(index: number) {
      queue[index].reject(new Error("Temporary settings failure"));
    },
  },
});
const rpc = <T,>(request: object) =>
  new Promise<T>((resolve, reject) =>
    queue.push({
      request: request as Record<string, unknown>,
      resolve: (value) => resolve(value as T),
      reject,
    }),
  );
function Fixture() {
  const [selected, setSelected] = useState<string | null>("financials");
  const [open, setOpen] = useState(false);
  const [saved, setSaved] = useState("");
  return (
    <main style={{ maxWidth: 720, margin: "0 auto" }}>
      <button aria-label="Open navigation">Navigation</button>
      <button id="settings-button" onClick={() => setOpen(true)}>
        Settings
      </button>
      <p id="saved-agent">{saved}</p>
      <ResearchPanel
        items={[item, prices]}
        receipts={[item.view, prices.view]}
        selectedId={selected}
        issues={[]}
        onSelect={setSelected}
      />
      <SettingsDialog
        open={open}
        onOpenChange={setOpen}
        rpc={rpc}
        onSaved={(value) => setSaved(value.selected)}
      />
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<Fixture />);
