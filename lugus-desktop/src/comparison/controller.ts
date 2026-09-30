import type { Rpc } from "../portfolio/api.ts";
import { comparisonApi } from "./api.ts";
import type {
  Input,
  Request,
  Record,
  Row,
  Source,
  Providers,
  Page,
  Job,
} from "./types.ts";
type State = {
  conversation: string;
  subjectHint: string;
  providers: Providers;
  records: Record[];
  selected: Record | null;
  rows: Row[];
  sources: Source[];
  job: Job | null;
  busy: boolean;
  loading: boolean;
  error: string;
};
const message = (e: unknown) =>
  e && typeof e === "object" && "message" in e ? String(e.message) : String(e);
export class ComparisonController {
  private api;
  private listeners = new Set<() => void>();
  private generation = 0;
  private selection = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;
  private sessions = new Map<
    string,
    {
      retry: { key: string; request: Request } | null;
      job: Job | null;
      submitting: boolean;
    }
  >();
  private session(conversation: string) {
    let session = this.sessions.get(conversation);
    if (!session) {
      session = { retry: null, job: null, submitting: false };
      this.sessions.set(conversation, session);
    }
    return session;
  }
  private state: State = {
    conversation: "",
    subjectHint: "",
    providers: { facts: [], resolution: [] },
    records: [],
    selected: null,
    rows: [],
    sources: [],
    job: null,
    busy: false,
    loading: false,
    error: "",
  };
  constructor(rpc: Rpc) {
    this.api = comparisonApi(rpc);
  }
  getSnapshot = () => this.state;
  subscribe = (f: () => void) => {
    this.listeners.add(f);
    return () => {
      this.listeners.delete(f);
    };
  };
  private patch(p: Partial<State>) {
    this.state = { ...this.state, ...p };
    for (const f of this.listeners) f();
  }
  private async pages<T>(
    kind: string,
    conversation: string,
    id?: string,
  ): Promise<T[]> {
    const items: T[] = [];
    let offset = 0;
    for (;;) {
      const p = await this.api<Page<T>>({
        kind,
        conversation,
        ...(id ? { id } : {}),
        offset,
      });
      items.push(...p.items);
      if (p.next_offset === null) return items;
      if (p.next_offset <= offset)
        throw Error("Saved comparison pagination did not advance.");
      offset = p.next_offset;
    }
  }
  open = async (conversation: string, subjectHint: string) => {
    const g = ++this.generation;
    this.selection++;
    if (this.timer) clearTimeout(this.timer);
    const session = this.session(conversation);
    this.patch({
      conversation,
      subjectHint,
      records: [],
      selected: null,
      rows: [],
      sources: [],
      job: session.job,
      busy: session.submitting || session.job?.state === "running",
      loading: true,
      error: "",
    });
    const [records, providers] = await Promise.allSettled([
      this.pages<Record>("list", conversation),
      this.api<Providers>({ kind: "providers" }),
    ]);
    if (g !== this.generation) return;
    this.patch({
      loading: false,
      records: records.status === "fulfilled" ? records.value : [],
      providers:
        providers.status === "fulfilled"
          ? providers.value
          : { facts: [], resolution: [] },
      error: records.status === "rejected" ? message(records.reason) : "",
    });
    if (records.status === "fulfilled" && records.value[0])
      await this.select(records.value[0].id);
    if (g === this.generation && session.job?.state === "running")
      await this.poll(session.job.id, g, this.selection);
  };
  select = async (id: string) => {
    const g = this.generation;
    const seq = ++this.selection;
    const conversation = this.state.conversation;
    this.patch({ loading: true, error: "" });
    try {
      const [selected, rows, sources] = await Promise.all([
        this.api<Record>({ kind: "read", conversation, id }),
        this.pages<Row>("rows", conversation, id),
        this.pages<Source>("sources", conversation, id),
      ]);
      if (g === this.generation && seq === this.selection)
        this.patch({ selected, rows, sources, loading: false });
    } catch (e) {
      if (g === this.generation && seq === this.selection)
        this.patch({ error: message(e), loading: false });
    }
  };
  create = async (input: Input) => {
    if (this.state.busy) return;
    const conversation = this.state.conversation;
    const session = this.session(conversation);
    const g = this.generation,
      selection = this.selection;
    const key = JSON.stringify(input);
    if (!session.retry || session.retry.key !== key)
      session.retry = {
        key,
        request: { ...input, request_id: crypto.randomUUID() },
      };
    const request = session.retry.request;
    session.submitting = true;
    this.patch({ busy: true, error: "" });
    try {
      const job = await this.api<Job>({ kind: "start", conversation, request });
      session.retry = null;
      session.job = job;
      session.submitting = false;
      if (!this.disposed && conversation === this.state.conversation)
        await this.acceptJob(
          job,
          this.generation,
          g === this.generation ? selection : this.selection,
        );
    } catch (e) {
      session.submitting = false;
      if (!this.disposed && conversation === this.state.conversation)
        this.patch({
          busy: session.job?.state === "running",
          error: message(e),
        });
    }
  };
  refresh = async (record: Record) => {
    const { request_id: _, ...input } = record.request;
    await this.create({ ...input, previous_id: record.id });
  };
  private schedule(id: string, g: number, selection: number, delay = 200) {
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      void this.poll(id, g, selection);
    }, delay);
  }
  private async acceptJob(job: Job, g: number, selection: number) {
    if (g !== this.generation) return;
    this.session(this.state.conversation).job = job;
    this.patch({ job, busy: job.state === "running", error: "" });
    if (this.timer) clearTimeout(this.timer);
    if (job.state === "running") {
      this.schedule(job.id, g, selection);
      return;
    }
    if (job.comparison_id) {
      try {
        const records = await this.pages<Record>(
          "list",
          this.state.conversation,
        );
        if (g !== this.generation) return;
        this.patch({ records });
        if (selection === this.selection) await this.select(job.comparison_id);
      } catch (e) {
        if (g === this.generation) this.patch({ error: message(e) });
      }
    } else if (job.error) this.patch({ error: job.error.message });
  }
  private async poll(id: string, g: number, selection: number) {
    if (g !== this.generation || this.disposed) return;
    try {
      const job = await this.api<Job>({
        kind: "status",
        conversation: this.state.conversation,
        job: id,
      });
      await this.acceptJob(job, g, selection);
    } catch (e) {
      if (g === this.generation) {
        this.patch({ busy: true, error: message(e) });
        this.schedule(id, g, selection, 1000);
      }
    }
  }
  cancel = async () => {
    const g = this.generation;
    const job = this.state.job;
    if (!job) return;
    try {
      await this.api({
        kind: "cancel",
        conversation: this.state.conversation,
        job: job.id,
      });
      if (g === this.generation) await this.poll(job.id, g, this.selection);
    } catch (e) {
      if (g === this.generation) this.patch({ error: message(e) });
    }
  };
  dispose = () => {
    this.disposed = true;
    this.generation++;
    this.selection++;
    if (this.timer) clearTimeout(this.timer);
    this.listeners.clear();
  };
}
