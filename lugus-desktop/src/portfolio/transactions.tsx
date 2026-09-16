import { useState } from "react";
import {
  CommandDialog,
  Field,
  SelectField,
  Check,
  today,
  orderValue,
} from "./forms";
import { decimalProduct } from "./format";
import type { TransactionRow } from "./types";
import type { MutationProps } from "./accounts";
export function TransactionDialog({
  accountId,
  nextOrder,
  original,
  ...props
}: MutationProps & {
  accountId: string | null;
  nextOrder: number;
  original?: TransactionRow;
}) {
  const e = original?.event;
  const k = e?.kind;
  const [id] = useState(e?.id ?? crypto.randomUUID());
  const [account, setAccount] = useState(
    original?.account_id ?? accountId ?? props.view.accounts[0]?.id ?? "",
  );
  const [kind, setKind] = useState(k?.kind ?? "deposit");
  const [date, setDate] = useState(e?.date ?? today());
  const [order, setOrder] = useState(String(e?.order ?? nextOrder));
  const [instrument, setInstrument] = useState(
    String(k?.instrument_id ?? props.view.instruments[0]?.id ?? ""),
  );
  const [amount, setAmount] = useState(String(k?.amount ?? ""));
  const [quantity, setQuantity] = useState(String(k?.quantity ?? ""));
  const [price, setPrice] = useState(String(k?.price ?? ""));
  const [reportedGross, setGross] = useState(String(k?.gross ?? ""));
  const [fees, setFees] = useState(String(k?.fees ?? "0"));
  const [override, setOverride] = useState(k?.gross_overridden === true);
  const [numerator, setNumerator] = useState(String(k?.numerator ?? "2"));
  const [denominator, setDenominator] = useState(String(k?.denominator ?? "1"));
  let gross = reportedGross;
  if (!override && quantity && price) {
    try {
      gross = decimalProduct(quantity, price);
    } catch {
      gross = "";
    }
  }
  return (
    <CommandDialog
      title={original ? "Correct transaction" : "Add transaction"}
      {...props}
      envelope={{
        portfolio_id: props.view.id,
        expected_revision: props.view.revision,
      }}
      mutation={() => {
        if (!account) throw new Error("Add an account first.");
        if (["buy", "sell", "dividend", "split"].includes(kind) && !instrument)
          throw new Error("Add and select an instrument first.");
        if (kind === "split") {
          const n = orderValue(numerator);
          const d = orderValue(denominator);
          if (!n || !d) throw new Error("Split ratio must be positive.");
          return original
            ? {
                kind: "replace_split",
                action_id: k?.action_id,
                date,
                order: orderValue(order),
                numerator: n,
                denominator: d,
              }
            : {
                kind: "apply_split",
                instrument_id: instrument,
                date,
                order: orderValue(order),
                numerator: n,
                denominator: d,
              };
        }
        const payload = ["buy", "sell"].includes(kind)
          ? {
              kind,
              instrument_id: instrument,
              quantity,
              price,
              gross,
              fees,
              gross_overridden: override,
            }
          : kind === "dividend"
            ? { kind, instrument_id: instrument, amount }
            : { kind, amount };
        return {
          kind: "edit_events",
          account_id: account,
          edits: [
            {
              kind: original ? "replace" : "append",
              event: { id, date, order: orderValue(order), kind: payload },
            },
          ],
        };
      }}
    >
      <SelectField
        label="Account"
        value={account}
        onChange={setAccount}
        disabled={!!original}
        options={props.view.accounts.map((a) => ({
          value: a.id,
          label: a.name,
        }))}
      />
      <SelectField
        label="Activity"
        value={kind}
        onChange={setKind}
        disabled={k?.kind === "split"}
        options={[
          "deposit",
          "buy",
          "sell",
          "withdrawal",
          "dividend",
          "fee",
          "split",
        ].map((value) => ({
          value,
          label: value[0].toUpperCase() + value.slice(1),
        }))}
      />
      <Field
        label="Effective date"
        type="date"
        value={date}
        onChange={setDate}
      />
      <Field
        label="Same-day order"
        type="number"
        min="0"
        step="1"
        value={order}
        onChange={setOrder}
      />
      {["buy", "sell", "dividend", "split"].includes(kind) && (
        <SelectField
          label="Instrument"
          value={instrument}
          onChange={setInstrument}
          options={props.view.instruments.map((i) => ({
            value: i.id,
            label: `${i.symbol} — ${i.name}`,
          }))}
        />
      )}
      {["deposit", "withdrawal", "dividend", "fee"].includes(kind) && (
        <Field label="Amount (USD)" value={amount} onChange={setAmount} />
      )}
      {["buy", "sell"].includes(kind) && (
        <section>
          <Field label="Shares" value={quantity} onChange={setQuantity} />
          <Field
            label="Execution price (USD)"
            value={price}
            onChange={setPrice}
          />
          <Field
            label="Gross trade amount (USD)"
            value={gross}
            onChange={setGross}
            readOnly={!override}
          />
          <Field label="Fees (USD)" value={fees} onChange={setFees} />
          <Check
            label="Use broker-reported gross amount"
            checked={override}
            onChange={(value) => {
              if (value) setGross(gross);
              setOverride(value);
            }}
          />
        </section>
      )}
      {kind === "split" && (
        <section>
          <p>
            This action adjusts this instrument across all accounts in the
            portfolio.
          </p>
          <Field
            label="New shares"
            type="number"
            value={numerator}
            onChange={setNumerator}
          />
          <Field
            label="Old shares"
            type="number"
            value={denominator}
            onChange={setDenominator}
          />
        </section>
      )}
    </CommandDialog>
  );
}
export function VoidDialog({
  row,
  ...props
}: MutationProps & {
  row: TransactionRow;
}) {
  return (
    <CommandDialog
      title="Void transaction"
      {...props}
      envelope={{
        portfolio_id: props.view.id,
        expected_revision: props.view.revision,
      }}
      mutation={() =>
        row.event.kind.kind === "split"
          ? { kind: "void_split", action_id: row.event.kind.action_id }
          : {
              kind: "edit_events",
              account_id: row.account_id,
              edits: [{ kind: "void", id: row.event.id }],
            }
      }
    >
      <p>
        Remove this {row.event.kind.kind} from accounting? Its original record
        remains in audit history. Linked stock splits are removed across all
        affected accounts. Later transactions will be revalidated.
      </p>
    </CommandDialog>
  );
}
