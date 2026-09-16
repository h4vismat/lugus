import { useState } from "react";
import { ArrowLeft, BookOpen, TrendingUp } from "lucide-react";
import { Button } from "../components/ui/button";
import { Label } from "../components/ui/label";
import {
  NativeSelect,
  NativeSelectOption,
} from "../components/ui/native-select";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "../components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "../components/ui/table";
import { selectedResearch } from "../data";
import {
  chartGeometry,
  exactNumber,
  sameCompany,
  type Identifier,
} from "../state";
import type { ResearchView, View, Price } from "../types";
import { FinancialsPanel } from "./financials";
export type ResearchTab = "overview" | "financials" | "filings";
const identity = (item: ResearchView): Identifier | undefined =>
  item.binding?.record.company.candidate.identifier ??
  item.data.rows.flatMap((row) =>
    row.kind === "fact"
      ? row.group.candidates.slice(0, 1).map((c) => c.value.company)
      : row.kind === "filing" || row.kind === "reported_fact"
        ? [row.evidence.value.company]
        : row.kind === "candidate"
          ? [row.entry.candidate.identifier]
          : [],
  )[0] ??
  queryCompany(item);
function queryCompany(item: ResearchView): Identifier | undefined {
  const query = item.data.header.query;
  const scope = query.scope as Record<string, unknown> | undefined;
  const company = (query.company ?? scope?.company) as Identifier | undefined;
  return company &&
    typeof company.namespace === "string" &&
    typeof company.value === "string"
    ? company
    : undefined;
}
function title(item: ResearchView, all: ResearchView[]) {
  const company = identity(item);
  const known = all.flatMap((v) =>
    v.data.rows.flatMap((row) =>
      row.kind === "candidate" &&
      sameCompany(row.entry.candidate.identifier, company)
        ? [row.entry.candidate.name]
        : [],
    ),
  )[0];
  const price = item.data.rows.find(
    (row): row is Price => row.kind === "price",
  );
  return (
    item.binding?.record.company.candidate.name ??
    all.find(
      (v) =>
        v.binding &&
        sameCompany(v.binding.record.company.candidate.identifier, company),
    )?.binding?.record.company.candidate.name ??
    known ??
    (company ? company.value : price?.evidence.value.instrument.value) ??
    "Research evidence"
  );
}
export function viewLabel(item: ResearchView, all: ResearchView[]) {
  const kind = item.data.header.kind;
  const concept =
    typeof item.data.header.query.concept === "string"
      ? item.data.header.query.concept.replace(/([a-z])([A-Z])/g, "$1 $2")
      : "";
  return `${title(item, all)} · ${kind === "prices" ? "Price history" : kind === "facts" ? concept || "Financials" : kind === "filings" ? "Filings" : kind === "resolution" ? "Company matches" : "Document"}`;
}
type Preferences = { range: string; tab: ResearchTab };
function readPreferences(id: string | null): Preferences {
  try {
    const saved = JSON.parse(
      id ? (localStorage.getItem(`lugus:view:${id}`) ?? "null") : "null",
    );
    return {
      range: ["1M", "6M", "1Y", "ALL"].includes(saved?.range)
        ? saved.range
        : "ALL",
      tab: ["overview", "financials", "filings"].includes(saved?.tab)
        ? saved.tab
        : "overview",
    };
  } catch {
    return { range: "ALL", tab: "overview" };
  }
}
function PricePanel({
  item,
  range,
  onRange,
}: {
  item: ResearchView;
  range: string;
  onRange: (range: string) => void;
}) {
  const all = item.data.rows
    .filter((r): r is Price => r.kind === "price")
    .sort((a, b) => a.evidence.value.date.localeCompare(b.evidence.value.date));
  const end = all.at(-1)?.evidence.value.date;
  const days =
    range === "1M"
      ? 31
      : range === "6M"
        ? 183
        : range === "1Y"
          ? 366
          : Infinity;
  const rows = all.filter(
    (r) =>
      !end ||
      Date.parse(end) - Date.parse(r.evidence.value.date) <= days * 86400000,
  );
  const latest = rows.at(-1);
  const currencies = new Set(rows.map((p) => p.evidence.value.currency));
  const plot = chartGeometry(
    rows.map((p) => ({ date: p.evidence.value.date, value: p.value })),
  );
  return (
    <section className="space-y-3">
      <p className="price-value text-3xl font-semibold tabular-nums">
        {latest
          ? `${exactNumber(latest.value)} ${latest.evidence.value.currency}`
          : "No prices available"}
      </p>
      <p className="price-description text-sm text-muted-foreground">
        {latest
          ? `Daily close · ${latest.evidence.value.date}`
          : "No price observations were returned."}
      </p>
      <div
        className="range-controls flex gap-1"
        role="group"
        aria-label="Price chart range"
      >
        {["1M", "6M", "1Y", "ALL"].map((key) => (
          <Button
            size="sm"
            variant={key === range ? "secondary" : "ghost"}
            aria-pressed={key === range}
            key={key}
            onClick={() => onRange(key)}
          >
            {key}
          </Button>
        ))}
      </div>
      {currencies.size <= 1 ? (
        <>
          <svg
            viewBox="0 0 380 190"
            className="price-chart w-full"
            role="img"
            aria-label={`Daily closing price history, ${rows[0]?.evidence.value.date ?? ""} to ${end ?? ""}; exact values in the table below`}
          >
            {[20, 70, 120, 170].map((y) => (
              <line
                key={y}
                x1="20"
                x2="360"
                y1={y}
                y2={y}
                className="stroke-border"
                strokeWidth="1"
              />
            ))}
            {plot.segments.map((d, index) => (
              <path
                key={index}
                d={d}
                fill="none"
                className="stroke-primary"
                strokeWidth="2"
              />
            ))}
            {plot.points.map((p, index) => (
              <circle
                key={index}
                cx={p.x}
                cy={p.y}
                r={plot.points.length === 1 ? 4 : 2}
                className="fill-primary"
              >
                <title>
                  {p.date}: {p.value}
                </title>
              </circle>
            ))}
          </svg>
          <div className="chart-dates flex justify-between text-xs text-muted-foreground">
            <span>{rows[0]?.evidence.value.date ?? ""}</span>
            <span>{end ?? ""}</span>
          </div>
        </>
      ) : (
        <p className="evidence-note">
          Different currencies are present; inspect the exact observations
          below.
        </p>
      )}
      <details>
        <summary className="cursor-pointer text-sm text-primary">
          Exact price observations
        </summary>
        <Table>
          <TableHeader>
            <TableRow>
              {["Trading date", "Close", "Currency"].map((label) => (
                <TableHead key={label}>{label}</TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row, index) => (
              <TableRow key={index}>
                <TableCell>{row.evidence.value.date}</TableCell>
                <TableCell className="tabular-nums">
                  {exactNumber(row.value)}
                </TableCell>
                <TableCell>{row.evidence.value.currency}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </details>
      <p className="evidence-note">
        {item.data.header.coverage?.completeness ?? "Coverage not specified"} ·{" "}
        {all.length} of {item.data.header.row_count} saved observations shown.
        Prices are historical, not live quotes.
      </p>
      {latest && (
        <p className="source-note">
          Retrieved {latest.evidence.value.retrieved_at.slice(0, 10)} ·{" "}
          {item.data.header.provider.instance_id}
        </p>
      )}
    </section>
  );
}
function FactsPanel({ items }: { items: ResearchView[] }) {
  const complete = items.find(
    (item) => item.data.header.policy === "all-reported-facts-v1",
  );
  const facts = items.flatMap((item) =>
    item.data.rows.flatMap((row) =>
      row.kind === "fact"
        ? [{ fact: row, provider: item.data.header.provider.instance_id }]
        : [],
    ),
  );
  return (
    <section className="financial-section mt-6 space-y-3">
      <h3 className="font-semibold">Reported financials</h3>
      {complete ? (
        <p className="evidence-note">
          {complete.data.header.row_count.toLocaleString()} financial
          observations are available. Open the Financials tab for the latest
          filings, every saved metric and full history.
        </p>
      ) : facts.length ? (
        <>
          {facts.map(({ fact, provider }, index) => {
            const candidate = fact.group.candidates[0]?.value;
            const p = fact.group.period;
            return (
              <div
                className="metric grid gap-1 rounded-lg border bg-card p-3"
                key={index}
              >
                <span>
                  {candidate?.label ??
                    candidate?.concept.replace(/([a-z])([A-Z])/g, "$1 $2") ??
                    "Reported metric"}
                </span>
                <strong
                  className={
                    fact.group.conflict
                      ? "conflict text-destructive"
                      : "tabular-nums"
                  }
                >
                  {fact.group.conflict
                    ? "Conflicting reports"
                    : `${exactNumber(fact.group.value)}${candidate ? " " + candidate.unit : ""}`}
                </strong>
                <small className="text-muted-foreground">
                  {p.kind === "instant" ? p.date : `${p.start} – ${p.end}`} ·
                  filed {fact.group.filed} · {provider}
                </small>
              </div>
            );
          })}
          <p className="source-note">
            Source-reported metrics and periods. Missing values remain missing;
            these are not assembled financial statements.
          </p>
        </>
      ) : (
        <p className="evidence-note">
          No reported fundamentals are open for this company yet. Ask the agent
          to inspect its financials.
        </p>
      )}
    </section>
  );
}
interface ResearchPanelProps {
  items: ResearchView[];
  receipts: View[];
  selectedId: string | null;
  issues: string[];
  onSelect: (id: string) => void;
  onBackToQuestion?: () => void;
  inspection?: { viewId: string; tab: ResearchTab; sequence: number } | null;
}
export function ResearchPanel(props: ResearchPanelProps) {
  const inspection =
    props.inspection?.viewId === props.selectedId ? props.inspection : null;
  return (
    <ResearchSurface
      key={`${props.selectedId ?? "unselected"}:${inspection?.sequence ?? 0}`}
      {...props}
    />
  );
}
function ResearchSurface({
  items,
  receipts,
  selectedId,
  issues,
  onSelect,
  onBackToQuestion,
  inspection,
}: ResearchPanelProps) {
  const [preferences, setPreferences] = useState(() => ({
    ...readPreferences(selectedId),
    ...(inspection?.viewId === selectedId ? { tab: inspection.tab } : {}),
  }));
  const updatePreferences = (patch: Partial<Preferences>) =>
    setPreferences((current) => {
      const next = { ...current, ...patch };
      if (selectedId)
        try {
          localStorage.setItem(
            `lugus:view:${selectedId}`,
            JSON.stringify(next),
          );
        } catch {
          /* Saved research remains available without browser storage. */
        }
      return next;
    });
  const selected = selectedResearch(items, selectedId);
  const company = selected ? identity(selected) : undefined;
  const related = selected
    ? company
      ? items.filter((i) => sameCompany(identity(i), company))
      : [selected]
    : [];
  const documentSelected =
    receipts.find((view) => view.id === selectedId)?.kind === "document";
  const name = selected
    ? title(selected, items)
    : documentSelected
      ? "Saved filing document"
      : "Company research";
  const binding = selected?.binding ?? related.find((i) => i.binding)?.binding;
  const listing = binding
    ? `${binding.record.listing.ticker.value} · ${binding.record.listing.exchange?.value ?? "Exchange not specified"}`
    : company
      ? `${company.namespace} · ${company.value}`
      : (selected?.data.header.provider.instance_id ??
        "Charts & financial information");
  const price =
    selected?.data.header.kind === "prices"
      ? selected
      : related.find((i) => i.data.header.kind === "prices");
  const financial =
    selected?.data.header.kind === "facts"
      ? selected
      : [...related]
          .filter((i) => i.data.header.kind === "facts")
          .sort((a, b) =>
            b.data.header.created_at.localeCompare(a.data.header.created_at),
          )[0];
  const filings = related.flatMap((item) =>
    item.data.rows.flatMap((row) =>
      row.kind === "filing" ? [row.evidence.value] : [],
    ),
  );
  const notes = [
    ...new Set([
      ...issues,
      ...related.flatMap((item) => [
        ...item.data.header.limitations,
        ...item.data.header.conflicts,
        ...(item.error ? [item.error] : []),
      ]),
    ]),
  ];
  return (
    <div className="flex min-h-0 min-w-0 flex-col">
      <div className="research-toolbar">
        <span className="eyebrow">Research library</span>
        {onBackToQuestion && (
          <Button variant="ghost" size="sm" onClick={onBackToQuestion}>
            <ArrowLeft size={14} aria-hidden="true" />
            Back to question
          </Button>
        )}
      </div>
      <header className="company-header flex items-center gap-3 p-5">
        <div
          id="company-icon"
          className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-secondary text-lg font-semibold text-primary"
        >
          {selected ? (
            name.slice(0, 1).toUpperCase()
          ) : (
            <TrendingUp className="size-5" />
          )}
        </div>
        <div className="min-w-0">
          <h2
            id="company-name"
            tabIndex={-1}
            className="font-display text-xl break-words"
          >
            {name}
          </h2>
          <p id="company-listing" className="text-xs text-muted-foreground">
            {listing}
          </p>
        </div>
      </header>
      {selected && (
        <div className="research-snapshot">
          <div className="snapshot-heading">
            <BookOpen size={13} aria-hidden="true" />
            <span>Saved snapshot</span>
          </div>
          <dl>
            <div>
              <dt>Source</dt>
              <dd>{selected.data.header.provider.instance_id}</dd>
            </div>
            <div>
              <dt>Saved</dt>
              <dd>
                <time dateTime={selected.data.header.created_at}>
                  {selected.data.header.created_at
                    .replace("T", " · ")
                    .replace(/Z$/, " UTC")}
                </time>
              </dd>
            </div>
          </dl>
          <p>Ask a new research question for updated evidence.</p>
        </div>
      )}
      <div id="view-picker" hidden={receipts.length < 2} className="px-5 pb-4">
        <Label htmlFor="research-view" className="mb-2">
          Saved research
        </Label>
        <NativeSelect
          id="research-view"
          value={selectedId ?? receipts[0]?.id ?? ""}
          onChange={(e) => onSelect(e.target.value)}
          className="w-full"
        >
          {receipts.map((view) => {
            const item = items.find((r) => r.view.id === view.id);
            return (
              <NativeSelectOption key={view.id} value={view.id}>
                {item
                  ? viewLabel(item, items)
                  : view.kind === "document"
                    ? "Saved filing document"
                    : "Unavailable research"}
              </NativeSelectOption>
            );
          })}
        </NativeSelect>
      </div>
      <Tabs
        value={preferences.tab}
        onValueChange={(tab) => updatePreferences({ tab: tab as ResearchTab })}
        className="min-h-0 min-w-0 gap-0"
      >
        <TabsList
          className="mx-5 mb-4 grid grid-cols-3"
          aria-label="Research sections"
        >
          {(["overview", "financials", "filings"] as const).map((tab) => (
            <TabsTrigger id={`tab-${tab}`} key={tab} value={tab}>
              {tab[0].toUpperCase() + tab.slice(1)}
            </TabsTrigger>
          ))}
        </TabsList>
        <div id="research-content" className="min-w-0 overflow-auto px-5 pb-6">
          {documentSelected ? (
            <p className="evidence-note">
              A saved filing document is selected as message context. Document
              reading is available through the agent.
            </p>
          ) : !selected ? (
            <div className="research-empty">
              <BookOpen size={24} aria-hidden="true" />
              <h3>
                {selectedId ? "Research unavailable" : "Follow the evidence"}
              </h3>
              <p className="evidence-note">
                {selectedId
                  ? "This saved view could not be opened. It remains message context until you remove it from the composer."
                  : "Ask about a company. Its saved price history, financials, and filings will appear here alongside your conversation."}
              </p>
              {onBackToQuestion && (
                <Button variant="outline" size="sm" onClick={onBackToQuestion}>
                  {selectedId
                    ? "Return to conversation"
                    : "Ask about a company"}
                  <ArrowLeft size={14} aria-hidden="true" />
                </Button>
              )}
            </div>
          ) : (
            <>
              <TabsContent value="overview" aria-labelledby="tab-overview">
                {price ? (
                  <PricePanel
                    item={price}
                    range={preferences.range}
                    onRange={(range) => updatePreferences({ range })}
                  />
                ) : (
                  <p className="evidence-note">
                    No price chart is open for this company yet. Ask the agent
                    for its price history.
                  </p>
                )}
                <FactsPanel items={related} />
              </TabsContent>
              <TabsContent value="financials" aria-labelledby="tab-financials">
                <FinancialsPanel
                  key={financial?.view.id ?? "empty"}
                  item={financial}
                />
              </TabsContent>
              <TabsContent
                value="filings"
                aria-labelledby="tab-filings"
                className="space-y-3"
              >
                {filings.length ? (
                  filings.map((filing, index) => (
                    <article
                      className="filing rounded-lg border bg-card p-4"
                      key={index}
                    >
                      <strong>{filing.form}</strong>
                      <p className="text-sm text-muted-foreground">
                        Filed {filing.filed}
                      </p>
                      <p className="mt-2 break-all text-xs">
                        {filing.source_url}
                      </p>
                    </article>
                  ))
                ) : (
                  <p className="evidence-note">
                    No filing list is open for this company. Ask the agent to
                    inspect its filings.
                  </p>
                )}
              </TabsContent>
              {selected.data.header.error && (
                <p className="evidence-note">
                  {selected.data.header.error.message}
                </p>
              )}
              {selected.data.header.kind === "resolution" &&
                selected.data.rows.map((row, index) =>
                  row.kind === "candidate" ? (
                    <div key={index} className="mt-4">
                      <h3 className="font-semibold">
                        {row.entry.candidate.name}
                      </h3>
                      <p className="evidence-note">
                        {row.entry.candidate.identifier.namespace}:{" "}
                        {row.entry.candidate.identifier.value}
                      </p>
                    </div>
                  ) : null,
                )}
            </>
          )}
          {notes.map((note) => (
            <p key={note} className="evidence-note mt-3">
              {note}
            </p>
          ))}
        </div>
      </Tabs>
    </div>
  );
}
