import { useState } from "react";
import { Button } from "../components/ui/button";
import { CommandDialog, Field, SelectField, Check, today } from "./forms";
import type { PortfolioApi } from "./api";
import type {
  Account,
  Instrument,
  Opening,
  OpeningLot,
  Receipt,
  View,
} from "./types";
export interface MutationProps {
  api: PortfolioApi;
  view: View;
  onClose: () => void;
  onSaved: (r: Receipt) => Promise<void>;
}
export function AccountDialog({
  api,
  view,
  onClose,
  onSaved,
  existing,
}: {
  existing?: Account;
} & MutationProps) {
  const [name, setName] = useState(existing?.name ?? "");
  const [start, setStart] = useState(existing?.start ?? today());
  const [mode, setMode] = useState(existing?.opening?.kind ?? "full_history");
  const [cash, setCash] = useState(
    existing?.opening?.kind === "existing" ? existing.opening.cash : "0",
  );
  const [lots, setLots] = useState<OpeningLot[]>(
    existing?.opening?.kind === "existing" ? existing.opening.lots : [],
  );
  const [error, setError] = useState("");
  const patch = (id: string, p: Partial<OpeningLot>) =>
    setLots((l) => l.map((row) => (row.id === id ? { ...row, ...p } : row)));
  return (
    <CommandDialog
      title={existing ? "Correct starting balances" : "Add account"}
      api={api}
      envelope={{ portfolio_id: view.id, expected_revision: view.revision }}
      onClose={onClose}
      onSaved={onSaved}
      mutation={() => {
        const opening: Opening =
          mode === "existing"
            ? {
                kind: "existing",
                cash,
                lots: lots.map((lot, index) => ({
                  ...lot,
                  acquired: lot.date_assumed ? start : lot.acquired,
                  tie_order: existing ? lot.tie_order : index,
                  simplified: lot.simplified || lot.date_assumed,
                })),
              }
            : { kind: "full_history" };
        return existing
          ? {
              kind: "change_setup",
              account_id: existing.id,
              start,
              opening,
              edits: [],
            }
          : { kind: "create_account", name, start, opening, events: [] };
      }}
    >
      <Field
        label="Account name"
        value={name}
        onChange={setName}
        disabled={!!existing}
      />
      <Field label="Start date" value={start} onChange={setStart} type="date" />
      <SelectField
        label="Starting point"
        value={mode}
        onChange={(v) => setMode(v as typeof mode)}
        options={[
          { value: "full_history", label: "Enter full transaction history" },
          { value: "existing", label: "Start from existing holdings" },
        ]}
      />
      <p className="portfolio-muted">
        {mode === "full_history"
          ? "The account starts at zero. Enter deposits, purchases, sales and other transactions from this date in chronological order."
          : "Enter cash and the lots you still own at the start of this date. Opening balances do not count as new purchases or income."}
      </p>
      {mode === "existing" && (
        <section>
          <Field label="Opening cash (USD)" value={cash} onChange={setCash} />
          <div className="portfolio-lots">
            {lots.map((lot) => (
              <fieldset key={lot.id}>
                <legend>Remaining purchase lot</legend>
                <SelectField
                  label="Instrument"
                  value={lot.instrument_id}
                  onChange={(instrument_id) => patch(lot.id, { instrument_id })}
                  options={view.instruments.map((i) => ({
                    value: i.id,
                    label: `${i.symbol} — ${i.name}`,
                  }))}
                />
                <Field
                  label="Original purchase date"
                  type="date"
                  value={lot.acquired}
                  disabled={lot.date_assumed}
                  onChange={(acquired) => patch(lot.id, { acquired })}
                />
                <Field
                  label="Remaining shares"
                  value={lot.quantity}
                  onChange={(quantity) => patch(lot.id, { quantity })}
                />
                <Field
                  label="Remaining total cost basis, including fees (USD)"
                  value={lot.basis}
                  onChange={(basis) => patch(lot.id, { basis })}
                />
                <Check
                  label="Aggregate basis only — simplified FIFO history"
                  checked={lot.simplified}
                  onChange={(simplified) => patch(lot.id, { simplified })}
                />
                <Check
                  label="Original purchase date unknown"
                  checked={lot.date_assumed}
                  onChange={(date_assumed) =>
                    patch(lot.id, {
                      date_assumed,
                      ...(date_assumed ? { simplified: true } : {}),
                    })
                  }
                />
                <Button
                  type="button"
                  variant="outline"
                  onClick={() =>
                    setLots((rows) => rows.filter((r) => r.id !== lot.id))
                  }
                >
                  Remove lot
                </Button>
              </fieldset>
            ))}
          </div>
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              if (!view.instruments.length) {
                setError("Add an instrument before entering opening lots.");
                return;
              }
              setLots((rows) => [
                ...rows,
                {
                  id: crypto.randomUUID(),
                  instrument_id: view.instruments[0].id,
                  acquired: start,
                  tie_order: rows.length,
                  quantity: "",
                  basis: "",
                  simplified: false,
                  date_assumed: false,
                },
              ]);
            }}
          >
            Add purchase lot
          </Button>
          <p role="status">{error}</p>
        </section>
      )}
    </CommandDialog>
  );
}
export function InstrumentDialog(props: MutationProps) {
  const [name, setName] = useState("");
  const [symbol, setSymbol] = useState("");
  const [kind, setKind] = useState("stock");
  return (
    <CommandDialog
      title="Add instrument"
      {...props}
      envelope={{
        portfolio_id: props.view.id,
        expected_revision: props.view.revision,
      }}
      mutation={() => ({
        kind: "create_instrument",
        name,
        symbol,
        asset_kind: kind,
      })}
    >
      <Field label="Name" value={name} onChange={setName} />
      <Field label="Ticker" value={symbol} onChange={setSymbol} />
      <SelectField
        label="Asset type"
        value={kind}
        onChange={setKind}
        options={[
          { value: "stock", label: "Stock" },
          { value: "etf", label: "ETF" },
        ]}
      />
      <p>Currency: USD</p>
    </CommandDialog>
  );
}
export function BindingDialog({
  instrument,
  providers,
  ...props
}: MutationProps & {
  instrument: Instrument;
  providers: {
    instance_id: string;
    name: string;
  }[];
}) {
  const [provider, setProvider] = useState(
    instrument.binding?.instance_id ?? providers[0]?.instance_id ?? "",
  );
  const [namespace, setNamespace] = useState(
    instrument.binding?.native_id.namespace ?? "yahoo:symbol",
  );
  const [symbol, setSymbol] = useState(
    instrument.binding?.native_id.value ?? instrument.symbol,
  );
  return (
    <CommandDialog
      title={`Price source for ${instrument.symbol}`}
      {...props}
      envelope={{
        portfolio_id: props.view.id,
        expected_revision: props.view.revision,
      }}
      mutation={() => ({
        kind: "bind_instrument",
        instrument_id: instrument.id,
        instance_id: provider,
        native_id: { namespace, value: symbol },
      })}
    >
      <SelectField
        label="Provider"
        value={provider}
        onChange={setProvider}
        options={providers.map((p) => ({
          value: p.instance_id,
          label: `${p.name} (${p.instance_id})`,
        }))}
      />
      <Field
        label="Provider symbol namespace"
        value={namespace}
        onChange={setNamespace}
      />
      <Field label="Provider symbol" value={symbol} onChange={setSymbol} />
    </CommandDialog>
  );
}
