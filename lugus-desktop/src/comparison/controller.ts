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
  private retry: { key: string; request: Request } | null = null;
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
    this.retry = null;
    this.patch({
      conversation,
      subjectHint,
      records: [],
      selected: null,
      rows: [],
      sources: [],
      job: null,
      busy: false,
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
    const g = this.generation;
    const key = JSON.stringify(input);
    if (!this.retry || this.retry.key !== key)
      this.retry = {
        key,
        request: { ...input, request_id: crypto.randomUUID() },
      };
    const request = this.retry.request;
    const conversation = this.state.conversation;
    this.patch({ busy: true, error: "" });
    try {
      const job = await this.api<Job>({ kind: "start", conversation, request });
      if (g !== this.generation) return;
      this.retry = null;
      await this.acceptJob(job, g, this.selection);
    } catch (e) {
      if (g === this.generation) this.patch({ busy: false, error: message(e) });
    }
  };
  refresh = async (record: Record) => {
    this.retry = null;
    const { request_id: _, ...input } = record.request;
    await this.create({ ...input, previous_id: record.id });
  };
  private async acceptJob(job: Job, g: number, selection: number) {
    if (g !== this.generation) return;
    this.patch({ job, busy: job.state === "running" });
    if (job.state === "running") {
      this.timer = setTimeout(() => {
        void this.poll(job.id, g, selection);
      }, 200);
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
    try {
      const job = await this.api<Job>({
        kind: "status",
        conversation: this.state.conversation,
        job: id,
      });
      await this.acceptJob(job, g, selection);
    } catch (e) {
      if (g === this.generation) {
        this.patch({ busy: false, error: message(e) });
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
    this.generation++;
    this.selection++;
    if (this.timer) clearTimeout(this.timer);
    this.listeners.clear();
  };
}
