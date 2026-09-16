import type { CompanySummary } from "../companies/types";
import { readSavedWindow, readSavedFacts } from "../data.ts";
import { acceptsResult, type Selection } from "../state.ts";
import type {
  Conversation,
  Message,
  Run,
  Page,
  Activity,
  Workspace,
  View,
  Dataset,
  Binding,
  ResearchView,
} from "../types";
import type { Rpc, PortfolioApi } from "../portfolio/api";
import type { View as PortfolioView, Snapshot } from "../portfolio/types";
import type { AgentSettings } from "../settings";
export const terminal = (run: Run) =>
  ["completed", "failed", "interrupted"].includes(run.status);
const errorText = (error: unknown) =>
  typeof error === "object" && error !== null && "message" in error
    ? String(error.message)
    : String(error);
type Submission = {
  company: string | null;
  review: boolean;
  conversation: string;
  request: string;
  text: string;
  company_hint: string | null;
  selected: {
    kind: "view" | "portfolio";
    id: string;
  }[];
};
type Monitor = {
  run: Run;
  text: string;
  activity: string;
  offset: number;
};
export type ChatState = {
  company: CompanySummary | null;
  review: boolean;
  selection: Selection;
  chats: Conversation[];
  nextChats: number | null;
  messages: Message[];
  workspace: Workspace | null;
  research: ResearchView[];
  receipts: View[];
  issues: string[];
  run: Run | null;
  stream: string;
  activity: string;
  draft: string;
  hint: string;
  error: string;
  loading: boolean;
  submitting: boolean;
  ready: boolean;
  runtimeAvailable: boolean;
  online: boolean;
  connection: string;
  includeContext: boolean;
  portfolioContext: {
    id: string;
    conversation: string;
    date: string;
  } | null;
};
export class ChatController {
  private state: ChatState = {
    company: null,
    review: false,
    selection: { id: null, generation: 0 },
    chats: [],
    nextChats: 0,
    messages: [],
    workspace: null,
    research: [],
    receipts: [],
    issues: [],
    run: null,
    stream: "",
    activity: "",
    draft: "",
    hint: "",
    error: "",
    loading: false,
    submitting: false,
    ready: false,
    runtimeAvailable: false,
    online: false,
    connection: "Connecting…",
    includeContext: true,
    portfolioContext: null,
  };
  private listeners = new Set<() => void>();
  private drafts = new Map<string, string>();
  private reviews = new Map<string, boolean>();
  private hints = new Map<string, string>();
  private monitors = new Map<string, Monitor>();
  private timers = new Set<ReturnType<typeof setTimeout>>();
  private researchLoad = 0;
  private layoutRevision = "";
  private retry: Submission | null = null;
  private disposed = false;
  readonly rpc: Rpc;
  constructor(rpc: Rpc) {
    this.rpc = rpc;
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private patch(value: Partial<ChatState>) {
    if (this.disposed) return;
    this.state = { ...this.state, ...value };
    for (const listener of this.listeners) listener();
  }
  dispose() {
    this.disposed = true;
    for (const timer of this.timers) clearTimeout(timer);
    this.timers.clear();
    this.listeners.clear();
  }
  setError = (error: unknown) => this.patch({ error: errorText(error) });
  setDraft = (draft: string) => {
    this.drafts.set(this.state.selection.id ?? "new", draft);
    this.patch({ draft });
  };
  setHint = (hint: string) => {
    this.hints.set(this.state.selection.id ?? "new", hint);
    this.retry = null;
    this.patch({ hint });
  };
  removeContext = () => this.patch({ includeContext: false });
  removePortfolioContext = () => {
    this.retry = null;
    this.patch({ portfolioContext: null });
  };
  private current(origin: Selection) {
    return !this.disposed && acceptsResult(this.state.selection, origin);
  }
  private async pages<T>(
    operation: string,
    conversation: string,
  ): Promise<T[]> {
    let offset = 0;
    const output: T[] = [];
    while (true) {
      const page = await this.rpc<Page<T>>({ operation, conversation, offset });
      output.push(...page.items);
      if (page.next_offset === null) return output;
      if (page.next_offset <= offset)
        throw new Error("Saved history pagination did not advance.");
      offset = page.next_offset;
    }
  }
  async loadChats(append = false) {
    const page = await this.rpc<Page<Conversation>>({
      operation: "list",
      offset: append ? (this.state.nextChats ?? 0) : 0,
    });
    const old = this.state.chats;
    this.patch({
      nextChats: page.next_offset,
      chats: append
        ? [...old, ...page.items.filter((c) => !old.some((o) => o.id === c.id))]
        : [
            ...page.items,
            ...old.filter((c) => !page.items.some((f) => f.id === c.id)),
          ],
    });
  }
  updateCompanySelection = (company: CompanySummary) => {
    if (this.state.selection.id === company.conversation_id)
      this.patch({ company, hint: company.hint });
  };
  updateCompany = (company: CompanySummary) => {
    if (this.state.company?.id === company.id) this.patch({ company });
  };
  prepareReview = () => {
    if (!this.state.company) return;
    const question =
      "Review this company against my saved investment thesis. Check for new filings and reported financial results, compare them with the previous review, examine supporting and contradictory evidence, and identify unanswered questions. Cite sources and their dates; clearly state missing or stale evidence. Propose thesis changes for me to consider.";
    this.setDraft(
      this.state.draft ? `${this.state.draft}\n\n${question}` : question,
    );
    this.patch({ review: true });
  };
  cancelReview = () => this.patch({ review: false });
  async openCompany(company: CompanySummary) {
    const opening = this.openChat({
      id: company.conversation_id,
      workspace_id: "",
      title: company.name,
      created_at: "",
    });
    this.patch({
      company,
      hint: company.hint,
      review: this.reviews.get(company.conversation_id) ?? false,
    });
    await opening;
  }
  private select(id: string | null) {
    const key = this.state.selection.id ?? "new";
    this.reviews.set(key, this.state.review);
    this.drafts.set(key, this.state.draft);
    this.hints.set(key, this.state.hint);
    this.layoutRevision = "";
    this.patch({
      company: null,
      review: false,
      selection: { id, generation: this.state.selection.generation + 1 },
      messages: [],
      workspace: null,
      research: [],
      receipts: [],
      issues: [],
      run: null,
      stream: "",
      activity: "",
      includeContext: true,
      portfolioContext: null,
      draft: this.drafts.get(id ?? "new") ?? "",
      hint: this.hints.get(id ?? "new") ?? "",
      error: "",
      loading: !!id,
    });
  }
  newChat = () => {
    this.select(null);
    this.retry = null;
  };
  async openChat(chat: Conversation) {
    this.select(chat.id);
    const origin = { ...this.state.selection };
    try {
      const [messages, runs] = await Promise.all([
        this.pages<Message>("messages", chat.id),
        this.pages<Run>("runs", chat.id),
      ]);
      if (!this.current(origin)) return;
      const run = runs.find((r) => !terminal(r)) ?? runs.at(-1) ?? null;
      this.patch({ messages, run, loading: false });
      if (run && !terminal(run)) {
        const monitor = this.monitors.get(run.id);
        if (monitor)
          this.patch({ stream: monitor.text, activity: monitor.activity });
        this.startMonitor(run, chat.id);
      } else if (run?.status === "failed")
        this.setError(
          run.error?.message ??
            "The last research turn failed. You can send a new message to continue.",
        );
      await this.loadResearch(origin, true);
      if (this.current(origin))
        try {
          localStorage.setItem("lugus:last-chat", chat.id);
        } catch {}
    } catch (error) {
      if (this.current(origin))
        this.patch({ loading: false, error: errorText(error) });
    }
  }
  private async loadMessages(origin: Selection) {
    if (!origin.id) return;
    const messages = await this.pages<Message>("messages", origin.id);
    if (this.current(origin)) this.patch({ messages });
  }
  async loadResearch(origin: Selection, force = false) {
    if (!origin.id) return;
    const ticket = ++this.researchLoad;
    const workspace = await this.rpc<Workspace>({
      operation: "workspace",
      conversation: origin.id,
    });
    const current = () => this.current(origin) && ticket === this.researchLoad;
    if (!current()) return;
    const key = `${origin.id}:${workspace.revision}`;
    if (key === this.layoutRevision && !force) return;
    const cache = new Map(
      this.state.research.map((item) => [item.view.id, item]),
    );
    const research: ResearchView[] = [];
    const receipts: View[] = [];
    const issues: string[] = [];
    for (let start = 0; start < workspace.view_ids.length; start += 4) {
      const batch = await Promise.all(
        workspace.view_ids.slice(start, start + 4).map(async (id) => {
          try {
            const view = await this.rpc<View>({
              operation: "view",
              conversation: origin.id,
              view: id,
            });
            receipts.push(view);
            if (view.kind === "document") return null;
            if (cache.has(id) && !force) return cache.get(id)!;
            const first = await this.rpc<Dataset>({
              operation: "read",
              conversation: origin.id,
              view: id,
              offset: 0,
            });
            const read = async (offset: number) => {
              if (!current()) throw new Error("Research selection changed.");
              return this.rpc<Dataset>({
                operation: "read",
                conversation: origin.id,
                view: id,
                offset,
              });
            };
            const data = await (
              first.header.kind === "facts" ? readSavedFacts : readSavedWindow
            )(first, read);
            let binding: Binding | undefined;
            if (data.header.binding_id)
              try {
                binding = await this.rpc<Binding>({
                  operation: "binding",
                  conversation: origin.id,
                  binding: data.header.binding_id,
                });
              } catch (error) {
                issues.push(
                  `Company identity unavailable: ${errorText(error)}`,
                );
              }
            return { view, data, binding };
          } catch (error) {
            issues.push(`Saved research unavailable: ${errorText(error)}`);
            return null;
          }
        }),
      );
      if (!current()) return;
      research.push(
        ...batch.filter((item): item is ResearchView => item !== null),
      );
    }
    this.layoutRevision = key;
    this.patch({
      workspace,
      research,
      receipts: receipts.sort(
        (a, b) =>
          workspace.view_ids.indexOf(a.id) - workspace.view_ids.indexOf(b.id),
      ),
      issues,
    });
    const shown = research.find(
      (item) => item.view.id === workspace.selected_view_id,
    );
    if (shown)
      void this.rpc({
        operation: "presented",
        conversation: origin.id,
        view: shown.view.id,
        revision: shown.view.descriptor_revision,
        status: "presented",
      }).catch(() => {});
  }
  async selectView(view: string) {
    if (!this.state.selection.id || !this.state.workspace) return;
    const revision = this.state.workspace.revision;
    this.patch({
      selection: {
        ...this.state.selection,
        generation: this.state.selection.generation + 1,
      },
    });
    const origin = { ...this.state.selection };
    try {
      await this.rpc({
        operation: "select",
        conversation: origin.id,
        view,
        revision,
      });
      if (this.current(origin)) {
        this.patch({ includeContext: true });
        await this.loadResearch(origin, true);
      }
    } catch (error) {
      if (this.current(origin)) this.setError(error);
    }
  }
  private startMonitor(run: Run, conversation: string) {
    if (this.monitors.has(run.id) || this.disposed) return;
    const monitor: Monitor = {
      run,
      text: "",
      activity: "Researching your question…",
      offset: 0,
    };
    this.monitors.set(run.id, monitor);
    const poll = async () => {
      if (this.disposed) return;
      try {
        const [status, events] = await Promise.all([
          this.rpc<Run>({ operation: "status", conversation, run: run.id }),
          this.rpc<Page<Activity>>({
            operation: "activity",
            conversation,
            run: run.id,
            offset: monitor.offset,
          }),
        ]);
        if (this.disposed) return;
        monitor.run = status;
        for (const item of events.items) {
          monitor.offset++;
          try {
            const event = JSON.parse(item.data);
            if (item.kind === "preparation")
              monitor.activity =
                event.phase === "interpreting"
                  ? "Understanding your question…"
                  : event.phase === "retrieving"
                    ? "Preparing source evidence…"
                    : "Analyzing prepared evidence…";
            if (
              item.kind === "runtime" &&
              event.type === "text_delta" &&
              typeof event.text === "string"
            )
              monitor.text += event.text;
            if (item.kind === "runtime" && event.type === "tool_started")
              monitor.activity = toolLabel(String(event.name));
          } catch {}
        }
        if (this.state.selection.id === conversation) {
          const origin = { ...this.state.selection };
          this.patch({
            run: status,
            stream: monitor.text,
            activity: monitor.activity,
          });
          await this.loadResearch(origin).catch((error) => {
            if (this.current(origin))
              this.setError(`Research panel unavailable: ${errorText(error)}`);
          });
        }
        if (terminal(status)) {
          this.monitors.delete(run.id);
          if (this.state.selection.id === conversation) {
            const origin = { ...this.state.selection };
            await this.loadMessages(origin);
            if (this.current(origin)) {
              if (status.status === "failed")
                this.setError(
                  status.error?.message ??
                    "The agent could not complete this request.",
                );
              if (status.status === "interrupted")
                this.setError(
                  "Research stopped. Saved evidence is still available.",
                );
            }
          }
          await this.loadChats();
          return;
        }
        const timer = setTimeout(
          () => {
            this.timers.delete(timer);
            void poll();
          },
          events.next_offset === null ? 700 : 0,
        );
        this.timers.add(timer);
      } catch (error) {
        this.monitors.delete(run.id);
        if (this.state.selection.id === conversation)
          this.setError(
            `Could not read agent progress: ${errorText(error)}. Reopen this chat to reconnect; the turn is not replayed.`,
          );
      }
    };
    void poll();
  }
  async send() {
    const submittedDraft = this.state.draft;
    const text = submittedDraft.trim();
    const company_hint = this.state.hint.trim() || null;
    const company = this.state.company?.id ?? null;
    const review = this.state.review;
    if (
      !text ||
      this.state.submitting ||
      !this.state.ready ||
      !this.state.runtimeAvailable ||
      (this.state.run && !terminal(this.state.run))
    )
      return;
    this.patch({ submitting: true, error: "" });
    const origin = { ...this.state.selection };
    let submissionOrigin = origin;
    try {
      let conversation = origin.id;
      if (!conversation) {
        const created = await this.rpc<Conversation>({
          operation: "create",
          request: crypto.randomUUID(),
          title: Array.from(text.replace(/\s+/g, " ")).slice(0, 72).join(""),
        });
        this.patch({ chats: [created, ...this.state.chats] });
        if (!this.current(origin)) return;
        conversation = created.id;
        this.patch({
          selection: { id: conversation, generation: origin.generation + 1 },
        });
        this.hints.set(conversation, this.state.hint);
        this.hints.delete("new");
      }
      submissionOrigin = { ...this.state.selection };
      const selected: Submission["selected"] =
        this.state.includeContext && this.state.workspace?.selected_view_id
          ? [{ kind: "view", id: this.state.workspace.selected_view_id }]
          : [];
      if (this.state.portfolioContext?.conversation === conversation)
        selected.push({
          kind: "portfolio",
          id: this.state.portfolioContext.id,
        });
      const submission =
        this.retry?.conversation === conversation &&
        this.retry.company === company &&
        this.retry.review === review &&
        this.retry.text === text &&
        this.retry.company_hint === company_hint &&
        JSON.stringify(this.retry.selected) === JSON.stringify(selected)
          ? this.retry
          : {
              conversation,
              company,
              review,
              request: crypto.randomUUID(),
              text,
              company_hint,
              selected,
            };
      this.retry = submission;
      const { company: companyId, review: isReview, ...ordinary } = submission;
      const run = await this.rpc<Run>(
        companyId
          ? {
              operation: "company",
              command: {
                kind: "send",
                company: companyId,
                request: submission.request,
                text: submission.text,
                selected: submission.selected,
                review: isReview,
              },
            }
          : { operation: "send", ...ordinary },
      );
      this.retry = null;
      if (this.current(submissionOrigin)) {
        if (this.state.draft === submittedDraft) {
          this.setDraft("");
          this.patch({ review: false });
          this.reviews.delete(conversation);
          if (origin.id === null) this.drafts.delete("new");
        }
        this.patch({ run });
        await this.loadMessages(submissionOrigin);
      }
      if (!terminal(run)) this.startMonitor(run, conversation);
      await this.loadChats();
    } catch (error) {
      if (this.current(submissionOrigin)) this.setError(error);
    } finally {
      this.patch({ submitting: false });
    }
  }
  async stop() {
    const origin = { ...this.state.selection };
    if (!origin.id || !this.state.run) return;
    try {
      const run = await this.rpc<Run>({
        operation: "cancel",
        conversation: origin.id,
        run: this.state.run.id,
      });
      if (this.current(origin)) this.patch({ run });
    } catch (error) {
      if (this.current(origin)) this.setError(error);
    }
  }
  applySettings = (value: AgentSettings) =>
    this.patch({
      runtimeAvailable: value.runtime_available,
      ready: true,
      online: !value.offline,
      connection: value.offline
        ? "Offline"
        : value.runtime_available
          ? `${value.agents.find((a) => a.id === value.selected)?.label ?? "Agent"} ready`
          : "Agent unavailable",
      ...(value.runtime_available ? { error: "" } : {}),
    });
  async initialize() {
    const origin = { ...this.state.selection };
    try {
      const info = await this.rpc<{
        offline: boolean;
        runtime_available: boolean;
        agent?: string;
        startup_error?: string;
      }>({ operation: "info" });
      this.patch({
        ready: true,
        runtimeAvailable: info.runtime_available,
        online: !info.offline,
        connection: info.offline
          ? "Offline"
          : info.runtime_available
            ? info.agent === "claude_code"
              ? "Claude Code ready"
              : info.agent === "codex"
                ? "Codex ready"
                : "Agent ready"
            : "Agent not configured",
        error:
          info.startup_error ??
          (!info.runtime_available
            ? "Select an installed agent in Settings to start chatting."
            : ""),
      });
      await this.loadChats();
      if (!this.current(origin)) return;
      let last: string | null = null;
      try {
        last = localStorage.getItem("lugus:last-chat");
      } catch {}
      let chat = this.state.chats.find((c) => c.id === last);
      if (!chat && last)
        try {
          chat = await this.rpc<Conversation>({
            operation: "conversation",
            conversation: last,
          });
          this.patch({ chats: [...this.state.chats, chat] });
        } catch {}
      if (!this.current(origin)) return;
      chat ??= this.state.chats[0];
      if (chat) await this.openChat(chat);
    } catch (error) {
      this.patch({
        error: errorText(error),
        connection: "Connection unavailable",
      });
    }
  }
  async usePortfolio(
    api: PortfolioApi,
    view: PortfolioView,
    accountId: string | null,
    intent?: {
      name: string;
      symbol: string;
    },
  ) {
    let conversation = this.state.selection.id;
    const previousDraft = this.state.draft;
    const origin = { ...this.state.selection };
    if (!conversation) {
      const created = await this.rpc<Conversation>({
        operation: "create",
        request: crypto.randomUUID(),
        title: "Portfolio review",
      });
      this.patch({ chats: [created, ...this.state.chats] });
      if (!this.current(origin))
        throw new Error(
          "The selected conversation changed. Choose Research again.",
        );
      await this.openChat(created);
      conversation = created.id;
    }
    if (this.state.selection.id !== conversation)
      throw new Error(
        "The selected conversation changed. Choose Research again.",
      );
    const snapshotOrigin = { ...this.state.selection };
    const snapshot = await api<Snapshot>({
      kind: "snapshot",
      request: {
        request_id: crypto.randomUUID(),
        portfolio_id: view.id,
        account_id: accountId,
        expected_revision: view.revision,
        conversation_id: conversation,
      },
    });
    if (
      !this.current(snapshotOrigin) ||
      this.state.selection.id !== conversation
    )
      throw new Error(
        "The selected conversation changed. Choose Research again.",
      );
    this.patch({
      portfolioContext: {
        id: snapshot.id,
        conversation,
        date: snapshot.summary.as_of,
      },
    });
    if (intent) {
      const existing = this.state.draft || previousDraft;
      const question = `Review ${intent.name} (${intent.symbol}) in the context of my selected portfolio${accountId ? " account" : ""}. What should I understand about this holding's risks and role?`;
      this.setDraft(existing ? `${existing}\n\n${question}` : question);
      this.setHint(intent.symbol);
    } else {
      if (!this.state.draft && previousDraft) this.setDraft(previousDraft);
      this.setHint("");
    }
    this.retry = null;
  }
}
const toolLabel = (name: string) =>
  name.includes("resolve") || name.includes("lookup")
    ? "Finding company information…"
    : name.includes("bind")
      ? "Confirming the market instrument…"
      : name.includes("fetch") || name === "get_price_chart"
        ? "Retrieving source data…"
        : name.includes("open_view")
          ? "Opening research alongside the conversation…"
          : name.includes("dataset")
            ? "Inspecting financial evidence…"
            : name.includes("passage") || name.includes("text")
              ? "Reading filing evidence…"
              : "Working with research evidence…";
