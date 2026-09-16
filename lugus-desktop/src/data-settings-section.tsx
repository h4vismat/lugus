import { WebSearchSettingsSection } from "./web-search-settings-section";
import { useEffect, useRef, useState } from "react";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Label } from "./components/ui/label";

export type DataSettings = {
  revision: string;
  contact_name: string;
  contact_email: string;
  existing_identity: string | null;
  enabled: boolean;
  installed: boolean;
  ready: boolean;
  restart_required: boolean;
  offline: boolean;
};
const message = (error: unknown) =>
  error && typeof error === "object" && "message" in error
    ? String(error.message)
    : String(error);
export function DataSettingsSection({
  rpc,
  onBusyChange,
  agent,
}: {
  rpc: <T>(request: object) => Promise<T>;
  onBusyChange: (busy: boolean) => void;
  agent?: string;
}) {
  const [saved, setSaved] = useState<DataSettings | null>(null);
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [searchBusy, setSearchBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [reload, setReload] = useState(0);
  const alive = useRef(false);
  const rpcRef = useRef(rpc);
  rpcRef.current = rpc;
  useEffect(() => {
    let current = true;
    alive.current = true;
    setSaved(null);
    setError("");
    void rpcRef
      .current<DataSettings>({ operation: "data_settings" })
      .then((value) => {
        if (!current) return;
        setSaved(value);
        setName(value.contact_name);
        setEmail(value.contact_email);
        setEnabled(value.enabled);
      })
      .catch((e) => {
        if (current) setError(message(e));
      });
    return () => {
      current = false;
      alive.current = false;
    };
  }, [reload]);
  const dirty =
    !!saved &&
    (name.trim() !== saved.contact_name ||
      email.trim() !== saved.contact_email ||
      enabled !== saved.enabled);
  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!saved || busy || searchBusy || !dirty) return;
    setBusy(true);
    onBusyChange(true);
    setError("");
    setNotice("");
    try {
      const value = await rpcRef.current<DataSettings>({
        operation: "save_data_settings",
        revision: saved.revision,
        contact_name: name.trim(),
        contact_email: email.trim(),
        enabled,
      });
      if (alive.current) {
        setSaved(value);
        setName(value.contact_name);
        setEmail(value.contact_email);
        setEnabled(value.enabled);
        setNotice(
          value.restart_required
            ? "Saved. Restart Lugus to apply these data-source settings."
            : "Data-source settings saved.",
        );
      }
    } catch (e) {
      if (alive.current) setError(message(e));
    } finally {
      if (alive.current) setBusy(false);
      onBusyChange(false);
    }
  }
  const status = !saved
    ? "Loading data sources…"
    : saved.restart_required
      ? "Restart required — the current session still uses the previous settings."
      : saved.offline
        ? "Offline mode — external data retrieval is disabled."
        : saved.ready
          ? "SEC company research is ready."
          : !saved.installed
            ? "SEC provider installation is missing."
            : saved.enabled
              ? "SEC provider is configured but unavailable. Check its installation and restart Lugus."
              : "SEC company research is not enabled.";
  return (
    <section aria-label="Data sources" className="grid gap-4 py-2">
      <WebSearchSettingsSection rpc={rpc} agent={agent} disabled={busy} onBusyChange={(next) => {
        setSearchBusy(next);
        onBusyChange(next);
      }} />
      <div>
        <h3 className="font-semibold">SEC EDGAR</h3>
        <p className="text-sm text-muted-foreground">
          Company identification, SEC filings, and reported financial data.
          Market prices use your separately configured market-data provider.
        </p>
      </div>
      <p role="status" className="text-sm text-muted-foreground">
        {status}
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!saved && error && (
        <Button variant="outline" onClick={() => setReload((n) => n + 1)}>
          Retry data-source settings
        </Button>
      )}
      {saved && (
        <form onSubmit={(e) => void save(e)}>
          <fieldset disabled={busy || searchBusy} className="grid gap-3">
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={enabled}
              disabled={busy || (!saved.installed && !enabled)}
              onChange={(e) => setEnabled(e.target.checked)}
            />
            Enable SEC company research
          </label>
          <Label htmlFor="sec-contact-name">Contact name</Label>
          <Input
            id="sec-contact-name"
            autoComplete="name"
            value={name}
            maxLength={160}
            required={enabled}
            disabled={busy}
            onChange={(e) => setName(e.target.value)}
            placeholder="Your name or organization"
          />
          <Label htmlFor="sec-contact-email">Contact email</Label>
          <Input
            id="sec-contact-email"
            type="email"
            autoComplete="email"
            value={email}
            maxLength={254}
            required={enabled}
            disabled={busy}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="Your contact email"
          />
          <p className="text-sm text-muted-foreground">
            The SEC requires identifying contact information. Lugus stores it
            locally and sends it with requests to the SEC. This is separate from
            your agent sign-in.
          </p>
          {saved.existing_identity && !saved.contact_name && (
            <p className="text-sm text-muted-foreground">
              Existing contact identity: {saved.existing_identity}. Enter a name
              and email here to replace it.
            </p>
          )}
          {notice && (
            <p role="status" className="text-sm text-muted-foreground">
              {notice}
            </p>
          )}
          <Button type="submit" disabled={busy || !dirty}>
            {busy ? "Saving…" : "Save data sources"}
          </Button>
          </fieldset>
        </form>
      )}
    </section>
  );
}
