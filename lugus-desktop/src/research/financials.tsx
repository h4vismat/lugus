import { useMemo, useRef, useState } from "react";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
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
import {
  financialListing,
  financialValue,
  financialStatements,
  financialPeriodGroups,
  filingChoices,
  type FinancialListingRow,
  type FinancialStatement,
} from "../financials";
import type {
  FinancialPeriod,
  ResearchView,
  ReportedFinancialFact,
} from "../types";
const missing = (value: unknown) =>
  value === null || value === undefined || value === ""
    ? "Missing"
    : String(value);
const humanize = (name: string) => name.replace(/([a-z])([A-Z])/g, "$1 $2");
const periodText = (period: FinancialPeriod | undefined) =>
  !period
    ? "Missing"
    : period.kind === "instant"
      ? missing(period.date)
      : `${missing(period.start)} – ${missing(period.end)}`;
function ObservationDetails({ row }: { row: FinancialListingRow }) {
  const fact = row.observation;
  const fields: [string, unknown][] = [
    ["SEC concept", `${row.metric.namespace}:${row.metric.concept}`],
    ["Source label", fact?.label],
    ["Value", fact ? financialValue(fact.value) : null],
    ["Unit", row.metric.unit],
    ["Period type", fact?.period.kind],
    ["Reporting period", fact ? periodText(fact.period) : null],
    ["Form", fact?.form],
    ["Filed", fact?.filed],
    ["Fiscal year", fact?.fiscal_year],
    ["Fiscal period", fact?.fiscal_period],
    ["Accession", fact?.filing_id],
    ["Retrieved", fact?.retrieved_at],
    ["Source", fact?.source_url],
  ];
  return (
    <details className="financial-details mt-2">
      <summary className="cursor-pointer text-sm text-primary">
        Source details
      </summary>
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm whitespace-normal">
        {fields.map(([label, value]) => (
          <div className="contents" key={label}>
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="break-all">{missing(value)}</dd>
          </div>
        ))}
      </dl>
    </details>
  );
}
function CompleteFinancials({
  item,
  facts,
}: {
  item: ResearchView;
  facts: ReportedFinancialFact[];
}) {
  const [statement, setStatement] = useState<FinancialStatement>("income");
  const [filing, setFiling] = useState("latest");
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const container = useRef<HTMLDivElement>(null);
  const choices = useMemo(() => filingChoices(facts), [facts]);
  const listing = useMemo(
    () => financialListing(facts, { statement, filing, search, page }),
    [facts, statement, filing, search, page],
  );
  const turnPage = (next: number) => {
    setPage(next);
    container.current?.scrollTo({ top: 0 });
  };
  const scope =
    filing === "latest"
      ? `${listing.periodic ? "Latest saved annual and quarterly filings, including subsequent amendments" : "No annual or quarterly filings are saved; showing the newest available filing"}: ${listing.filings.map((f) => `${f.form} (${f.filed})`).join(", ")}.`
      : filing === "all"
        ? "All filings in this saved date range."
        : `Selected filing: ${listing.filings.map((f) => `${f.form} (${f.filed})`).join(", ")}.`;
  return (
    <Tabs
      value={statement}
      onValueChange={(value) => {
        setStatement(value as FinancialStatement);
        turnPage(0);
      }}
    >
      <TabsList
        className="financial-statement-nav h-auto flex flex-wrap"
        aria-label="Financial statement"
      >
        {financialStatements.map((choice) => (
          <TabsTrigger value={choice.id} key={choice.id}>
            {choice.label}
          </TabsTrigger>
        ))}
      </TabsList>
      <div className="financial-controls grid gap-3 py-4">
        <div className="grid gap-2">
          <Label htmlFor="financial-filings">Filings</Label>
          <NativeSelect
            id="financial-filings"
            aria-label="Financial filings"
            value={filing}
            onChange={(e) => {
              setFiling(e.target.value);
              setPage(0);
            }}
            className="w-full"
          >
            <NativeSelectOption value="latest">
              Latest annual &amp; quarterly
            </NativeSelectOption>
            <NativeSelectOption value="all">All filings</NativeSelectOption>
            {choices.map((choice) => (
              <NativeSelectOption key={choice.key} value={choice.key}>
                {choice.form} · {choice.filed} · {choice.id}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
        <div className="grid gap-2">
          <Label htmlFor="financial-search">Find a metric</Label>
          <Input
            id="financial-search"
            type="search"
            aria-label="Find a financial metric"
            placeholder="Name, SEC concept or unit"
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setPage(0);
            }}
          />
        </div>
      </div>
      <TabsContent value={statement}>
        <p className="financial-scope text-sm text-muted-foreground">{scope}</p>
        <h4 className="mt-4 font-semibold">
          {financialStatements.find((choice) => choice.id === statement)!.label}
        </h4>
        <p
          role="status"
          className="financial-count my-2 text-sm text-muted-foreground"
        >
          {listing.metricCount.toLocaleString()} metrics ·{" "}
          {listing.total.toLocaleString()} rows
          {listing.missingCount
            ? ` · ${listing.missingCount.toLocaleString()} not reported in selected filings`
            : ""}
        </p>
        <div
          ref={container}
          tabIndex={0}
          role="region"
          aria-label="Financial observations"
          className="financial-table-wrap max-h-[65vh] overflow-auto rounded-lg border"
        >
          {listing.total ? (
            <Table className="financial-table">
              <TableHeader>
                <TableRow>
                  {[
                    "Metric / source",
                    "Value",
                    "Unit",
                    "Reporting period",
                    "Filing",
                  ].map((label) => (
                    <TableHead scope="col" key={label}>
                      {label}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              {financialPeriodGroups(listing.rows).map((group) => (
                <TableBody key={group.key}>
                  <TableRow className="financial-period-heading bg-muted">
                    <TableHead colSpan={5} scope="rowgroup">
                      {group.period
                        ? `${group.period.kind === "instant" ? "As of" : "Period"} ${periodText(group.period)}`
                        : "Not reported in selected filings"}
                    </TableHead>
                  </TableRow>
                  {group.rows.map((row, index) => (
                    <TableRow key={`${row.metric.key}:${index}`}>
                      <TableCell className="min-w-52 align-top whitespace-normal">
                        <strong>
                          {row.observation?.label ||
                            row.metric.label ||
                            humanize(row.metric.concept)}
                        </strong>
                        <span className="financial-concept block text-xs text-muted-foreground">
                          {row.metric.namespace}:{row.metric.concept}
                        </span>
                        {row.observation && !row.observation.label && (
                          <span className="missing-field block text-xs">
                            Source label: Missing
                          </span>
                        )}
                        <ObservationDetails row={row} />
                      </TableCell>
                      <TableCell className="financial-value align-top tabular-nums">
                        {financialValue(row.observation?.value)}
                        {!row.observation && (
                          <small className="block">
                            Not reported in selected filings
                          </small>
                        )}
                      </TableCell>
                      <TableCell className="align-top">
                        {missing(row.metric.unit)}
                      </TableCell>
                      <TableCell className="align-top">
                        {periodText(row.observation?.period)}
                      </TableCell>
                      <TableCell className="align-top">
                        {row.observation
                          ? `${row.observation.form} · ${row.observation.filed}`
                          : "Missing"}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              ))}
            </Table>
          ) : (
            <p className="evidence-note p-4">
              {search.trim()
                ? "No metrics in this section match this search."
                : "No mapped metrics are available in this section. Check Other Metrics for additional SEC disclosures."}
            </p>
          )}
        </div>
        <div className="financial-pagination my-4 flex items-center justify-between gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={listing.page === 0}
            onClick={() => turnPage(listing.page - 1)}
          >
            Previous
          </Button>
          <span className="text-sm">
            Page {listing.page + 1} of{" "}
            {Math.max(1, Math.ceil(listing.total / listing.pageSize))}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={(listing.page + 1) * listing.pageSize >= listing.total}
            onClick={() => turnPage(listing.page + 1)}
          >
            Next
          </Button>
        </div>
        <p className="source-note">
          Saved filing dates: {missing(item.data.header.query.filed_from)} –{" "}
          {missing(item.data.header.query.filed_to)}.{" "}
          {facts.length.toLocaleString()} source observations. Retrieved{" "}
          {missing(facts[0]?.retrieved_at)} ·{" "}
          {item.data.header.provider.instance_id}
        </p>
        <p className="evidence-note">
          Metrics are categorized by standard SEC concepts, not the filing’s
          original statement layout. Unmapped metrics remain in Other Metrics.
          Each row retains its original period and filing. Missing means no
          observation in the selected filings, or an absent source field; it is
          never a zero. Annual, quarterly and year-to-date durations remain
          separate.
        </p>
      </TabsContent>
    </Tabs>
  );
}
export function FinancialsPanel({ item }: { item: ResearchView | undefined }) {
  if (!item)
    return (
      <p className="evidence-note">
        No saved financial data is available for this company. Ask Lugus to
        research its financials.
      </p>
    );
  const facts = item.data.rows.flatMap((row) =>
    row.kind === "reported_fact" ? [row.evidence.value] : [],
  );
  return (
    <section className="financial-section full-financials min-w-0 space-y-3">
      <h3 className="font-semibold">Reported financials</h3>
      {item.data.header.policy !== "all-reported-facts-v1" ? (
        <>
          <p className="evidence-note">
            This saved view contains selected metrics only. Start new financial
            research to open every available SEC metric.
          </p>
          {item.data.rows.map((row, index) => {
            if (row.kind !== "fact") return null;
            const fact = row.group.candidates[0]?.value;
            return (
              <div
                className="metric grid gap-1 rounded-lg border p-3"
                key={index}
              >
                <span>
                  {fact?.label ||
                    humanize(
                      fact?.concept ??
                        String(
                          item.data.header.query.concept ?? "Reported metric",
                        ),
                    )}
                </span>
                <strong>{financialValue(row.group.value)}</strong>
                <small>
                  {periodText(row.group.period)} · Filed{" "}
                  {missing(row.group.filed)} · {missing(fact?.unit)}
                </small>
                {!fact?.label && <small>Source label: Missing</small>}
                {row.group.conflict && (
                  <small className="conflict">
                    Conflicting reports: {row.group.conflict}
                  </small>
                )}
              </div>
            );
          })}
          {!item.data.rows.length && (
            <p className="evidence-note">
              {humanize(
                String(item.data.header.query.concept ?? "Financial values"),
              )}
              : Missing
            </p>
          )}
          {item.data.header.error && (
            <p className="evidence-note">{item.data.header.error.message}</p>
          )}
        </>
      ) : (
        <>
          {item.data.header.error && (
            <p className="evidence-note">
              Incomplete source retrieval: {item.data.header.error.message}
            </p>
          )}
          {facts.length !== item.data.header.row_count ? (
            <p className="evidence-note">
              The financial listing is incomplete. Reopen this research view to
              retry loading its saved data.
            </p>
          ) : !facts.length ? (
            <p className="evidence-note">
              Financial values: Missing. No observations were returned for this
              company and filing-date range.
            </p>
          ) : (
            <CompleteFinancials key={item.view.id} item={item} facts={facts} />
          )}
        </>
      )}
    </section>
  );
}
