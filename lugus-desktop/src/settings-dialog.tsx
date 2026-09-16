import { DataSettingsSection } from "./data-settings-section";
import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { Button } from "./components/ui/button";
import { Label } from "./components/ui/label";
import {
  NativeSelect,
  NativeSelectOption,
} from "./components/ui/native-select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./components/ui/dialog";
import { Skeleton } from "./components/ui/skeleton";
import {
  initialSettings,
  settingsReducer,
  canSaveSettings,
  type AgentKind,
  type AgentSettings,
  type SettingsAction,
} from "./settings";
interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  rpc: <T>(request: object) => Promise<T>;
  onSaved: (value: AgentSettings) => void;
}
const errorMessage = (error: unknown) =>
  typeof error === "object" && error !== null && "message" in error
    ? String(error.message)
    : String(error);
export function SettingsDialog({
  open,
  onOpenChange,
  rpc,
  onSaved,
}: SettingsDialogProps) {
  const [state, setState] = useState(initialSettings);
  const [section, setSection] = useState<"agent" | "data">("agent");
  const [dataVisited, setDataVisited] = useState(false);
  const [dataBusy, setDataBusy] = useState(false);
  const stateRef = useRef(state);
  const generation = useRef(0);
  const rpcRef = useRef(rpc);
  rpcRef.current = rpc;
  const savedRef = useRef(onSaved);
  savedRef.current = onSaved;
  const triggerRef = useRef<HTMLElement | null>(null);
  function update(action: SettingsAction) {
    stateRef.current = settingsReducer(stateRef.current, action);
    setState(stateRef.current);
  }
  useEffect(() => {
    const current = ++generation.current;
    if (!open) return;
    setSection("agent");
    setDataVisited(false);
    stateRef.current = initialSettings();
    setState(stateRef.current);
    void rpcRef
      .current<AgentSettings>({ operation: "agent_settings" })
      .then((value) => {
        if (current === generation.current) update({ type: "loaded", value });
      })
      .catch((error) => {
        if (current === generation.current)
          update({ type: "failed", message: errorMessage(error) });
      });
    return () => {
      generation.current++;
    };
  }, [open]);
  async function save() {
    const latest = stateRef.current;
    if (!canSaveSettings(latest) || !latest.draft) return;
    const current = generation.current;
    const agent = latest.draft;
    update({ type: "saving" });
    try {
      const value = await rpcRef.current<AgentSettings>({
        operation: "select_agent",
        agent,
      });
      if (current !== generation.current) return;
      update({ type: "saved", value });
      savedRef.current(value);
    } catch (error) {
      if (current === generation.current)
        update({ type: "failed", message: errorMessage(error) });
    }
  }
  const options = state.saved?.agents ?? [];
  const saving = state.status === "saving" || dataBusy;
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (stateRef.current.status !== "saving" && !dataBusy)
          onOpenChange(next);
      }}
    >
      <DialogContent
        id="settings"
        aria-describedby="settings-runtime"
        className="sm:max-w-lg max-h-[90vh] overflow-y-auto"
        showCloseButton={false}
        onOpenAutoFocus={() => {
          triggerRef.current =
            document.activeElement instanceof HTMLElement
              ? document.activeElement
              : null;
        }}
        onEscapeKeyDown={(event) => {
          if (stateRef.current.status === "saving" || dataBusy)
            event.preventDefault();
        }}
        onInteractOutside={(event) => {
          if (stateRef.current.status === "saving" || dataBusy)
            event.preventDefault();
        }}
        onCloseAutoFocus={(event) => {
          const captured = triggerRef.current;
          const trigger =
            captured?.isConnected &&
            captured !== document.body &&
            captured.getClientRects().length
              ? captured
              : document.querySelector<HTMLElement>(
                  'button[aria-label="Open navigation"]',
                );
          if (trigger) {
            event.preventDefault();
            trigger.focus();
          }
        }}
      >
        <Button
          id="settings-close"
          aria-label="Close settings"
          variant="ghost"
          size="icon"
          className="absolute right-3 top-3"
          disabled={saving}
          onClick={() => onOpenChange(false)}
        >
          <X className="size-4" />
        </Button>
        <DialogHeader>
          <DialogTitle>Application settings</DialogTitle>
          <DialogDescription id="settings-runtime">
            {state.saved?.offline
              ? "Offline mode is enabled. Reopen Lugus without LUGUS_OFFLINE to use an agent."
              : "Choose the agent for your next message. Each agent uses its own existing sign-in."}
          </DialogDescription>
        </DialogHeader>
        <nav className="flex gap-2" aria-label="Settings sections">
          <Button
            variant={section === "agent" ? "secondary" : "ghost"}
            disabled={saving}
            aria-current={section === "agent" ? "page" : undefined}
            onClick={() => setSection("agent")}
          >
            Agent
          </Button>
          <Button
            variant={section === "data" ? "secondary" : "ghost"}
            disabled={saving}
            aria-current={section === "data" ? "page" : undefined}
            onClick={() => {
              setSection("data");
              setDataVisited(true);
            }}
          >
            Data sources
          </Button>
        </nav>
        {open && dataVisited && (
          <div hidden={section !== "data"}>
            <DataSettingsSection rpc={rpc} onBusyChange={setDataBusy} agent={state.saved?.selected} />
          </div>
        )}
        <div hidden={section !== "agent"}>
          <div className="grid gap-3 py-2">
            <Label htmlFor="settings-agent">Agent</Label>
            {state.status === "loading" ? (
              <Skeleton className="h-9 w-full" />
            ) : (
              <NativeSelect
                id="settings-agent"
                aria-describedby="settings-detail"
                value={state.draft ?? ""}
                disabled={!state.saved || state.saved.offline || saving}
                onChange={(e) =>
                  update({ type: "choose", agent: e.target.value as AgentKind })
                }
                className="w-full"
              >
                {!options.length && (
                  <NativeSelectOption value="">
                    No agents available
                  </NativeSelectOption>
                )}
                {options.map((agent) => (
                  <NativeSelectOption
                    value={agent.id}
                    key={agent.id}
                    disabled={!agent.available}
                  >
                    {agent.label}
                    {agent.available ? "" : " — unavailable"}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            )}
            <p id="settings-detail" className="text-sm text-muted-foreground">
              {options.find((agent) => agent.id === state.draft)?.detail ??
                (state.status === "loading"
                  ? "Loading available agents…"
                  : "Agent availability could not be loaded.")}
            </p>
            <p
              id="settings-feedback"
              role={state.status === "error" ? "alert" : "status"}
              className={
                state.status === "error"
                  ? "settings-error text-sm text-destructive"
                  : "text-sm text-muted-foreground"
              }
            >
              {state.notice}
            </p>
          </div>
        </div>
        <DialogFooter>
          <Button
            id="settings-done"
            variant="outline"
            disabled={saving}
            onClick={() => onOpenChange(false)}
          >
            Done
          </Button>
          {section === "agent" && (
            <Button
              id="settings-save"
              disabled={!canSaveSettings(state)}
              onClick={() => void save()}
            >
              {saving ? "Saving…" : "Save agent"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
