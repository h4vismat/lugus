import { useEffect, useRef, useState } from "react";
import { Button } from "./components/ui/button";

type WebSearchSettings = {
  enabled: boolean;
  offline: boolean;
  runtime_available: boolean;
  agent: "codex" | "claude_code";
};
const message = (error: unknown) =>
  error && typeof error === "object" && "message" in error
    ? String(error.message)
    : String(error);

export function WebSearchSettingsSection({ rpc, onBusyChange, disabled, agent }: {
  rpc: <T>(request: object) => Promise<T>;
  onBusyChange: (busy: boolean) => void;
  disabled: boolean;
  agent?: string;
}) {
  const [saved, setSaved] = useState<WebSearchSettings | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [reload, setReload] = useState(0);
  const generation = useRef(0);
  const saving = useRef(false);
  const rpcRef = useRef(rpc);
  rpcRef.current = rpc;
  useEffect(() => {
    const current = ++generation.current;
    setSaved(null);
    setError("");
    setNotice("");
    void rpcRef.current<WebSearchSettings>({ operation: "web_search_settings" })
      .then((value) => {
        if (current !== generation.current) return;
        setSaved(value);
        setEnabled(value.enabled);
      })
      .catch((e) => {
        if (current === generation.current) setError(message(e));
      });
    return () => { generation.current++; };
  }, [reload, agent]);

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!saved || saving.current || disabled || saved.enabled === enabled) return;
    const current = generation.current;
    saving.current = true;
    setBusy(true);
    onBusyChange(true);
    setError("");
    setNotice("");
    try {
      const value = await rpcRef.current<WebSearchSettings>({
        operation: "save_web_search_settings", enabled,
      });
      if (current === generation.current) {
        setSaved(value);
        setEnabled(value.enabled);
        setNotice("Saved. Applies to new messages; current messages keep their settings.");
      }
    } catch (e) {
      if (current === generation.current) setError(message(e));
    } finally {
      saving.current = false;
      if (current === generation.current) setBusy(false);
      onBusyChange(false);
    }
  }
  const status = !saved
    ? "Loading internet search settings…"
    : saved.offline
      ? "Offline mode — internet search is unavailable."
      : !saved.runtime_available
        ? "Select an installed agent in Settings to use internet search."
        : !saved.enabled
          ? "Internet search is disabled."
          : `Internet search is enabled for ${saved.agent === "codex" ? "Codex" : "Claude Code"}. Availability depends on your model and provider.`;
  return (
    <section aria-label="Internet search" className="grid gap-3 border-b pb-5 mb-1">
      <div>
        <h3 className="font-semibold">Internet search</h3>
        <p className="text-sm text-muted-foreground">
          Search current news and public web sources using your selected agent's
          existing sign-in. Search requests may be sent to its provider.
          Answers link to web sources; pages are not saved as financial datasets.
        </p>
      </div>
      <p role="status" className="text-sm text-muted-foreground">{status}</p>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      {!saved && error && (
        <Button variant="outline" disabled={disabled} onClick={() => setReload((n) => n + 1)}>
          Retry internet search settings
        </Button>
      )}
      {saved && (
        <form onSubmit={(event) => void save(event)}>
          <fieldset disabled={busy || disabled} className="grid gap-3">
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
              Enable Internet search
            </label>
            {notice && <p role="status" className="text-sm text-muted-foreground">{notice}</p>}
            <Button type="submit" disabled={busy || disabled || saved.enabled === enabled}>
              {busy ? "Saving…" : "Save Internet search"}
            </Button>
          </fieldset>
        </form>
      )}
    </section>
  );
}
