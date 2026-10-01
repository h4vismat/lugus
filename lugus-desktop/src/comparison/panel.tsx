import { useEffect, useState, useSyncExternalStore } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import type { ComparisonController } from "./controller";
import type { Row } from "./types";
import { ComparisonTable } from "./table";
import { SourceDetails } from "./source-details";
export function ComparisonPanel({
  controller,
  open,
  onClose,
}: {
  controller: ComparisonController;
  open: boolean;
  onClose: () => void;
}) {
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const [first, setFirst] = useState("");
  const [second, setSecond] = useState("");
  const [end, setEnd] = useState(new Date().toISOString().slice(0, 10));
  const [years, setYears] = useState(3);
  const [basis, setBasis] = useState("contract_revenue_excluding_tax");
  const [question, setQuestion] = useState("");
  const [facts, setFacts] = useState("");
  const [resolution, setResolution] = useState("");
  const [row, setRow] = useState<Row | null>(null);
  useEffect(() => {
    setFirst(state.subjectHint);
    setSecond("");
    setFacts("");
    setResolution("");
    setRow(null);
  }, [state.conversation, state.subjectHint]);
  const available =
    state.providers.facts.length > 0 && state.providers.resolution.length > 0;
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!value) onClose();
      }}
    >
      <DialogContent className="comparison-dialog">
        <DialogHeader>
          <DialogTitle>Compare companies</DialogTitle>
          <DialogDescription>
            Saved financial comparisons with inspectable sources. Current
            disclosures for historical periods.
          </DialogDescription>
        </DialogHeader>
        <form
          className="comparison-form"
          onSubmit={(e) => {
            e.preventDefault();
            void controller.create({
              subjects: [
                { text: first, exchange: null },
                { text: second, exchange: null },
              ],
              period_end: end,
              years,
              revenue_basis: basis,
              question: question || null,
              facts_instance: facts || null,
              resolution_instance: resolution || null,
              previous_id: null,
            });
          }}
        >
          <label>
            First company
            <Input
              value={first}
              onChange={(e) => setFirst(e.target.value)}
              required
              placeholder="Company or ticker"
            />
          </label>
          <label>
            Second company
            <Input
              value={second}
              onChange={(e) => setSecond(e.target.value)}
              required
              placeholder="Company or ticker"
            />
          </label>
          <label>
            Reporting endpoint
            <Input
              type="date"
              value={end}
              max={new Date().toISOString().slice(0, 10)}
              onChange={(e) => setEnd(e.target.value)}
              required
            />
          </label>
          <label>
            Annual periods
            <select
              value={years}
              onChange={(e) => setYears(Number(e.target.value))}
            >
              {[1, 2, 3, 4, 5].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <label className="comparison-wide">
            Revenue definition
            <select value={basis} onChange={(e) => setBasis(e.target.value)}>
              <option value="contract_revenue_excluding_tax">
                Contract revenue excluding assessed tax
              </option>
              <option value="revenues">Reported revenues</option>
            </select>
          </label>
          {state.providers.facts.length > 1 && (
            <label>
              Financial provider
              <select
                required
                value={facts}
                onChange={(e) => setFacts(e.target.value)}
              >
                <option value="">Choose provider</option>
                {state.providers.facts.map((p) => (
                  <option key={p.instance_id}>{p.instance_id}</option>
                ))}
              </select>
            </label>
          )}
          {state.providers.resolution.length > 1 && (
            <label>
              Company lookup provider
              <select
                required
                value={resolution}
                onChange={(e) => setResolution(e.target.value)}
              >
                <option value="">Choose provider</option>
                {state.providers.resolution.map((p) => (
                  <option key={p.instance_id}>{p.instance_id}</option>
                ))}
              </select>
            </label>
          )}
          <label className="comparison-wide">
            Research question (optional)
            <Textarea
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              placeholder="What do you want to investigate?"
              maxLength={4096}
            />
          </label>
          <div className="comparison-wide comparison-actions">
            <Button type="submit" disabled={state.busy || !available}>
              Create comparison
            </Button>
            {state.busy && (
              <Button
                type="button"
                variant="outline"
                onClick={() => void controller.cancel()}
              >
                Cancel retrieval
              </Button>
            )}
            <span className="evidence-note">
              Creating or refreshing retrieves financial data.
            </span>
          </div>
        </form>
        {!available && !state.loading && (
          <p className="evidence-note">
            Configure a compatible SEC financial provider to create or refresh
            comparisons. Saved comparisons remain readable.
          </p>
        )}
        {state.error && (
          <p className="error-notice" role="alert">
            {state.error}
          </p>
        )}
        {state.busy && (
          <p role="status">Retrieving and calculating comparison…</p>
        )}
        {state.loading && <p role="status">Opening saved research…</p>}
        {state.records.length > 0 && (
          <div className="comparison-actions">
            <label>
              Saved comparison
              <select
                aria-label="Saved comparison"
                value={state.selected?.id ?? ""}
                onChange={(e) => void controller.select(e.target.value)}
              >
                <option value="" disabled>
                  Choose a saved version
                </option>
                {state.records.map((r, i) => (
                  <option key={r.id} value={r.id}>
                    Version {state.records.length - i} ·{" "}
                    {new Date(r.created_at).toLocaleString()} · {r.state}
                  </option>
                ))}
              </select>
            </label>
            <Button
              variant="outline"
              disabled={!state.selected || state.busy || !available}
              onClick={() => {
                if (state.selected) void controller.refresh(state.selected);
              }}
            >
              Refresh comparison
            </Button>
          </div>
        )}
        {state.selected && (
          <>
            <p className="evidence-note">
              {state.selected.request.years} annual periods through{" "}
              {state.selected.request.period_end} ·{" "}
              {state.selected.request.revenue_basis === "revenues"
                ? "Reported revenues"
                : "Contract revenue excluding assessed tax"}{" "}
              ·{" "}
              {state.selected.state === "partial"
                ? "Partial evidence"
                : "Requested calculations available"}
            </p>
            {state.selected.request.question && (
              <p>{state.selected.request.question}</p>
            )}
            <ComparisonTable
              record={state.selected}
              rows={state.rows}
              onInspect={setRow}
            />
            {state.selected.issues.map((i, n) => (
              <p className="evidence-note" key={n}>
                {i.detail}
              </p>
            ))}
            <p className="evidence-note">
              Select a period ending date to inspect exact evidence and
              formulas.
            </p>
          </>
        )}
        <SourceDetails
          row={row}
          sources={state.sources}
          onClose={() => setRow(null)}
        />
      </DialogContent>
    </Dialog>
  );
}
