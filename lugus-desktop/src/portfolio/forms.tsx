import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { NativeSelect } from "../components/ui/native-select";
import { Checkbox } from "../components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "../components/ui/dialog";
import type { PortfolioApi } from "./api";
import type { Command, Preview, Receipt } from "./types";
import { formatUsd } from "./format";
export function errorText(e: unknown) {
  return typeof e === "object" && e !== null && "message" in e
    ? String(e.message)
    : String(e);
}
export function today() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
export function orderValue(value: string) {
  const n = Number(value);
  if (!Number.isSafeInteger(n) || n < 0)
    throw new Error("Same-day order must be a nonnegative whole number.");
  return n;
}
export function Field({
  label,
  value,
  onChange,
  type = "text",
  ...props
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  type?: string;
  disabled?: boolean;
  readOnly?: boolean;
  required?: boolean;
  min?: string;
  step?: string;
}) {
  const id = useId();
  return (
    <div className="portfolio-field">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        required
        maxLength={type === "text" ? 256 : undefined}
        {...props}
      />
    </div>
  );
}
export function SelectField({
  label,
  value,
  onChange,
  options,
  disabled,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: {
    value: string;
    label: string;
  }[];
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="portfolio-field">
      <Label htmlFor={id}>{label}</Label>
      <NativeSelect
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </NativeSelect>
    </div>
  );
}
export function Check({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="portfolio-check">
      <Checkbox
        id={id}
        checked={checked}
        onCheckedChange={(v) => onChange(v === true)}
      />
      <Label htmlFor={id}>{label}</Label>
    </div>
  );
}
export function CommandDialog({
  title,
  api,
  envelope,
  mutation,
  children,
  onClose,
  onSaved,
}: {
  title: string;
  api: PortfolioApi;
  envelope: {
    portfolio_id: string | null;
    expected_revision: string;
  };
  mutation: () => Record<string, unknown>;
  children: ReactNode;
  onClose: () => void;
  onSaved: (r: Receipt) => Promise<void>;
}) {
  const [feedback, setFeedback] = useState("");
  const [busy, setBusy] = useState(false);
  const [label, setLabel] = useState("Preview");
  const pending = useRef<Command | null>(null);
  const fingerprint = useRef("");
  const version = useRef(0);
  const lock = useRef(false);
  const [trigger] = useState(() =>
    document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null,
  );
  let serializedMutation: string | null = null;
  try {
    serializedMutation = JSON.stringify(mutation());
  } catch {}
  useEffect(() => {
    if (pending.current && serializedMutation !== fingerprint.current)
      invalidate();
  }, [serializedMutation]);
  function invalidate() {
    version.current++;
    pending.current = null;
    setLabel("Preview");
    setFeedback("");
  }
  async function submit() {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    const wasReady = pending.current !== null;
    const origin = version.current;
    try {
      const change = mutation();
      const serialized = JSON.stringify(change);
      if (!pending.current || serialized !== fingerprint.current) {
        const request = {
          ...envelope,
          request_id: crypto.randomUUID(),
          mutation: change,
        };
        fingerprint.current = serialized;
        const preview = await api<Preview>({ kind: "preview", request });
        if (origin !== version.current) return;
        pending.current = request;
        const matches = preview.states.flatMap((s) => s.matches);
        setFeedback(
          `After this change\nCash: ${formatUsd(preview.view.valuation.cash)}\nRealized P&L: ${formatUsd(preview.view.realized)}${matches.length ? "\n\nFIFO sales:\n" + matches.map((m) => `${m.quantity} shares · basis ${formatUsd(m.basis)} · net proceeds ${formatUsd(m.net_proceeds)} · P&L ${formatUsd(m.realized)}${m.simplified ? " (simplified opening history)" : ""}`).join("\n") : ""}`,
        );
        setLabel("Save");
      } else {
        const receipt = await api<Receipt>({
          kind: "execute",
          request: pending.current,
        });
        onClose();
        await onSaved(receipt);
      }
    } catch (e) {
      setFeedback(errorText(e));
      const conflict =
        typeof e === "object" &&
        e !== null &&
        "kind" in e &&
        String(e.kind).toLowerCase() === "conflict";
      if (!wasReady || conflict) {
        pending.current = null;
        setLabel("Preview");
      } else setLabel("Retry save");
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent
        className="portfolio-dialog"
        onCloseAutoFocus={(e) => {
          e.preventDefault();
          if (trigger?.isConnected) trigger.focus();
        }}
        onEscapeKeyDown={(e) => {
          if (busy) e.preventDefault();
        }}
        onPointerDownOutside={(e) => {
          if (busy) e.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Review the accounting preview before saving this change.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
          onChange={invalidate}
        >
          <fieldset disabled={busy} className="portfolio-fields">
            {children}
          </fieldset>
          <pre className="portfolio-preview" role="status">
            {feedback}
          </pre>
          <div className="portfolio-actions">
            <Button
              type="button"
              variant="outline"
              disabled={busy}
              onClick={onClose}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? "Working…" : label}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
