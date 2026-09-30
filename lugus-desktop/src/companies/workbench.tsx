import { ComparisonController } from "../comparison/controller";
import { ComparisonPanel } from "../comparison/panel";
import { useEffect, useMemo, useRef, useState } from "react";
import { BookOpen, Plus, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { MessageText } from "../chat/messages";
import type { Company, CompanySummary, CompanyHistory } from "./types";
import type { Rpc } from "../portfolio/api";
import { companyApi } from "./api";

const readable = (error: unknown) =>
  error && typeof error === "object" && "message" in error
    ? String(error.message)
    : String(error);
const date = (text: string) => new Date(text).toLocaleString();
type Draft = { thesis: string; questions: string; revision: number };
export function CompanyWorkbench({
  rpc,
  selected,
  companies,
  onOpen,
  onChange,
  onResearch,
  onReview,
  refresh,
}: {
  rpc: Rpc;
  selected: CompanySummary | null;
  companies: CompanySummary[];
  onOpen: (company: CompanySummary) => void;
  onChange: (company: CompanySummary) => void;
  onResearch: (messageId?: string) => void;
  onReview: () => void;
  refresh: string;
}) {
  const api = companyApi(rpc);
  const comparison = useMemo(() => new ComparisonController(rpc), [rpc]);
  const [comparing, setComparing] = useState(false);
  useEffect(() => () => comparison.dispose(), [comparison]);
  useEffect(() => { setComparing(false); }, [selected?.id]);
  const [company, setCompany] = useState<Company | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const drafts = useRef(new Map<string, Draft>());
  const [history, setHistory] = useState<CompanyHistory | null>(null);
  const [tab, setTab] = useState("brief");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [hint, setHint] = useState("");
  const [load, setLoad] = useState(0);
  const creation = useRef<{
    name: string;
    hint: string;
    request: string;
  } | null>(null);
  const selectedId = useRef(selected?.id);
  selectedId.current = selected?.id;
  useEffect(() => {
    let alive = true;
    setCompany(null);
    setDraft(null);
    setHistory(null);
    setError("");
    setNotice("");
    if (!selected) return;
    const id = selected.id;
    api<Company>({ kind: "get", company: id })
      .then((value) => {
        if (!alive) return;
        setCompany(value);
        onChange(value);
        setDraft(
          drafts.current.get(id) ?? {
            thesis: value.thesis,
            questions: value.questions,
            revision: value.revision,
          },
        );
      })
      .catch((e) => {
        if (alive) setError(readable(e));
      });
    return () => {
      alive = false;
    };
  }, [selected?.id, selected?.revision, load]);
  useEffect(() => {
    let alive = true;
    if (!selected || tab !== "history") return;
    api<CompanyHistory>({ kind: "history", company: selected.id })
      .then((value) => {
        if (alive) setHistory(value);
      })
      .catch((e) => {
        if (alive) setError(readable(e));
      });
    return () => {
      alive = false;
    };
  }, [selected?.id, selected?.revision, tab, refresh, load]);
  const dirty =
    !!company &&
    !!draft &&
    (draft.thesis !== company.thesis || draft.questions !== company.questions);
  const edit = (value: Draft) => {
    if (!company) return;
    drafts.current.set(company.id, value);
    setDraft(value);
    setNotice("");
  };
  const save = async () => {
    if (!company || !draft || busy) return;
    const id = company.id;
    setBusy(true);
    setError("");
    try {
      const result = await api<Company>({
        kind: "save",
        company: id,
        ...draft,
      });
      drafts.current.delete(id);
      onChange(result);
      if (selectedId.current === id) {
        setCompany(result);
        setDraft({
          thesis: result.thesis,
          questions: result.questions,
          revision: result.revision,
        });
        setNotice("Research brief saved.");
      }
    } catch (e) {
      if (selectedId.current === id) setError(readable(e));
    } finally {
      setBusy(false);
    }
  };
  const create = async () => {
    if (busy || !name.trim() || !hint.trim()) return;
    setBusy(true);
    setError("");
    const fields = { name: name.trim(), hint: hint.trim() };
    const request =
      creation.current?.name === fields.name &&
      creation.current.hint === fields.hint
        ? creation.current.request
        : crypto.randomUUID();
    creation.current = { ...fields, request };
    try {
      const result = await api<Company>({ kind: "create", request, ...fields });
      onChange(result);
      onOpen(result);
      setAdding(false);
      setName("");
      setHint("");
      creation.current = null;
    } catch (e) {
      setError(readable(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="company-workbench" aria-label="Company workbench">
      <div className="company-toolbar">
        <div>
          <span className="eyebrow">Your research, over time</span>
          <h2>{selected?.name ?? "Companies"}</h2>
          <p>
            {selected
              ? `${selected.hint} · Saved research workspace`
              : "Keep a thesis, follow the evidence, and return with better questions."}
          </p>
        </div>
        <Button variant="outline" onClick={() => setAdding(!adding)}>
          <Plus size={16} /> Add company
        </Button>
      </div>
      {error && (
        <p role="alert" className="error-notice">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="evidence-note">
          {notice}
        </p>
      )}
      {(adding || companies.length === 0) && (
        <form
          className="company-create"
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <label>
            Company name
            <Input
              aria-label="Company name"
              value={name}
              maxLength={200}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Apple"
            />
          </label>
          <label>
            Company or ticker hint
            <Input
              aria-label="Company or ticker hint"
              value={hint}
              maxLength={200}
              onChange={(e) => setHint(e.target.value)}
              placeholder="e.g. AAPL · Nasdaq"
            />
          </label>
          <Button disabled={busy || !name.trim() || !hint.trim()} type="submit">
            {busy ? "Saving…" : "Create company"}
          </Button>
          <p className="evidence-note">
            Adding a company saves a workspace. Source identity is confirmed
            when you request research.
          </p>
        </form>
      )}
      {!selected && (
        <div className="company-grid">
          {companies.map((c) => (
            <button
              key={c.id}
              className="company-card"
              onClick={() => onOpen(c)}
            >
              <BookOpen size={20} />
              <strong>{c.name}</strong>
              <span>{c.hint}</span>
              <small>Open saved research →</small>
            </button>
          ))}
        </div>
      )}
      {selected && !company && !error && (
        <p role="status">Opening company research…</p>
      )}
      {company && company.id === selected?.id && draft && (
        <>
          <div className="company-actions">
            <Badge variant="secondary">Brief revision {company.revision}</Badge>
            <span className="evidence-note">
              Saved {date(company.updated_at)}
            </span>
            <Button variant="outline" onClick={() => { void comparison.open(company.conversation_id, company.hint); setComparing(true); }}>Compare companies</Button>
            <ComparisonPanel controller={comparison} open={comparing} onClose={() => setComparing(false)} />
            <Button onClick={() => onResearch()}>Research with agent</Button>
            <Button
              variant="outline"
              disabled={dirty || busy}
              onClick={onReview}
            >
              <RefreshCw size={15} /> Review thesis
            </Button>
          </div>
          {dirty && (
            <p className="evidence-note">
              You have unsaved edits. Save your brief before preparing a review.
              Chat uses the last saved version.
            </p>
          )}
          <nav className="company-tabs" aria-label="Company sections">
            {["brief", "findings", "history"].map((t) => (
              <Button
                key={t}
                variant={tab === t ? "secondary" : "ghost"}
                aria-current={tab === t ? "page" : undefined}
                onClick={() => setTab(t)}
              >
                {t === "brief"
                  ? "Thesis & questions"
                  : t === "findings"
                    ? `Accepted findings (${company.findings.length})`
                    : "Review history"}
              </Button>
            ))}
          </nav>
          {tab === "brief" && (
            <form
              className="company-brief"
              onSubmit={(e) => {
                e.preventDefault();
                void save();
              }}
            >
              <label htmlFor="company-thesis">Investment thesis</label>
              <p className="evidence-note">
                Why is this business interesting? What must remain true, and
                what would change your mind?
              </p>
              <Textarea
                id="company-thesis"
                value={draft.thesis}
                disabled={busy}
                onChange={(e) => edit({ ...draft, thesis: e.target.value })}
                placeholder="Write your working investment hypothesis…"
                rows={8}
              />
              <label htmlFor="company-questions">Open research questions</label>
              <Textarea
                id="company-questions"
                value={draft.questions}
                disabled={busy}
                onChange={(e) => edit({ ...draft, questions: e.target.value })}
                placeholder="What do you need to understand next?"
                rows={5}
              />
              <div className="company-actions">
                <Button type="submit" disabled={busy || !dirty}>
                  {busy ? "Saving…" : "Save research brief"}
                </Button>
                <Button
                  variant="ghost"
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    drafts.current.delete(company.id);
                    setLoad((n) => n + 1);
                  }}
                >
                  Reload saved brief
                </Button>
              </div>
              <p className="evidence-note">
                The agent receives your saved thesis, questions, recent accepted
                findings and a previous review excerpt. Your thesis remains
                yours to edit.
              </p>
            </form>
          )}
          {tab === "findings" && (
            <div className="company-findings">
              {company.findings.length === 0 ? (
                <p className="company-empty">
                  Save a useful agent answer as a finding. You can edit the
                  excerpt before accepting it into your research.
                </p>
              ) : (
                [...company.findings].reverse().map((f, i) => (
                  <article
                    className="company-finding"
                    key={`${f.message_id}-${i}`}
                  >
                    <MessageText text={f.text} />
                    <small>
                      Accepted {date(f.created_at)} · {f.evidence.length}{" "}
                      associated evidence references
                    </small>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => onResearch(f.message_id)}
                    >
                      Open original answer
                    </Button>
                  </article>
                ))
              )}
            </div>
          )}
          {tab === "history" && (
            <div className="company-history">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setLoad((n) => n + 1)}
              >
                Refresh history
              </Button>
              <h3>Thesis reviews</h3>
              {!history ? (
                <p role="status">Loading history…</p>
              ) : (
                <>
                  {!history.reviews.length && (
                    <p className="company-empty">
                      No reviews yet. Review thesis prepares a question for the
                      agent; you choose when to send it.
                    </p>
                  )}
                  {history.reviews.map((r, i) => (
                    <details
                      key={r.run_id ?? `pending-${i}`}
                      className="company-finding"
                    >
                      <summary>
                        {date(r.created_at)} · {r.status.replaceAll("_", " ")}
                      </summary>
                      {r.text && <MessageText text={r.text} />}{" "}
                      {r.error && <p role="alert">{r.error.message}</p>}
                      <details>
                        <summary>Research brief used for this review</summary>
                        <ReviewBrief raw={r.brief} />
                      </details>
                    </details>
                  ))}
                  <h3>Brief revisions</h3>
                  {history.revisions.map((r) => (
                    <details key={r.revision} className="company-finding">
                      <summary>
                        Revision {r.revision} · {date(r.updated_at)}
                      </summary>
                      <h4>Thesis</h4>
                      <p className="company-preserve">
                        {r.thesis || "No thesis saved."}
                      </p>
                      <h4>Open questions</h4>
                      <p className="company-preserve">
                        {r.questions || "No questions saved."}
                      </p>
                    </details>
                  ))}
                  {history.next_offset !== null && (
                    <Button
                      variant="outline"
                      onClick={async () => {
                        const id = company.id;
                        try {
                          const next = await api<CompanyHistory>({
                            kind: "history",
                            company: id,
                            offset: history.next_offset,
                          });
                          if (selectedId.current === id)
                            setHistory({
                              ...next,
                              reviews: [...history.reviews, ...next.reviews],
                              revisions: [
                                ...history.revisions,
                                ...next.revisions,
                              ],
                            });
                        } catch (e) {
                          if (selectedId.current === id) setError(readable(e));
                        }
                      }}
                    >
                      Older history
                    </Button>
                  )}
                </>
              )}
            </div>
          )}
        </>
      )}
    </section>
  );
}

function ReviewBrief({ raw }: { raw: string }) {
  try {
    const brief = JSON.parse(raw) as {
      company: string;
      revision: number;
      updated_at: string;
      thesis: string;
      open_questions: string;
      accepted_findings: { text: string }[];
      omitted_findings: number;
      previous_review?: { excerpt: string; truncated: boolean } | null;
      previous_review_omitted?: boolean;
    };
    return (
      <div className="company-preserve">
        <p className="evidence-note">
          {brief.company} · revision {brief.revision} · saved{" "}
          {date(brief.updated_at)}
        </p>
        <h4>Investment thesis</h4>
        <p>{brief.thesis || "No thesis saved."}</p>
        <h4>Open questions</h4>
        <p>{brief.open_questions || "No questions saved."}</p>
        <h4>Accepted findings included</h4>
        {brief.accepted_findings.map((f, i) => (
          <p key={i}>{f.text}</p>
        ))}
        {brief.omitted_findings > 0 && (
          <p className="evidence-note">
            {brief.omitted_findings} older findings were omitted to keep the
            brief concise.
          </p>
        )}
        {brief.previous_review && (
          <>
            <h4>Previous review excerpt</h4>
            <p>{brief.previous_review.excerpt}</p>
            {brief.previous_review.truncated && (
              <p className="evidence-note">
                Excerpt only; the full answer remains in review history.
              </p>
            )}
          </>
        )}
        {brief.previous_review_omitted && (
          <p className="evidence-note">
            The previous review excerpt did not fit in this brief.
          </p>
        )}
      </div>
    );
  } catch {
    return (
      <p className="error-notice">
        The saved review brief could not be displayed.
      </p>
    );
  }
}
