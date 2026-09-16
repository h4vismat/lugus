import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../components/ui/button";
import { Card } from "../components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "../components/ui/table";
import type { HistoryController } from "./history-controller";
import type { ChartMode } from "./history-state";
import type { PortfolioHistoryPage, HistoryEvidencePage } from "./types";
import {
  historyGeometry,
  sampleHistory,
  hasFlow,
  portfolioPlotValue,
} from "./history-geometry";
import { formatMoney, formatPercent } from "./format";
import { errorText } from "./forms";
export function Performance({ history: h }: { history: HistoryController }) {
  const [mode, setMode] = useState<ChartMode>("return");
  const [inspection, setInspection] = useState(-1);
  const [dataOpen, setDataOpen] = useState(false);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [data, setData] = useState<PortfolioHistoryPage | null>(null);
  const [sources, setSources] = useState<HistoryEvidencePage | null>(null);
  const [dataError, setDataError] = useState("");
  const [sourceError, setSourceError] = useState("");
  const dataGeneration = useRef(0);
  const sourceGeneration = useRef(0);
  const sampled = useMemo(() => sampleHistory(h.rows, 650), [h.rows]);
  const geometry = useMemo(
    () => historyGeometry(sampled, mode),
    [sampled, mode],
  );
  const full = useMemo(() => historyGeometry(h.rows, mode), [h.rows, mode]);
  const index = Math.max(
    0,
    Math.min(
      h.rows.length - 1,
      inspection < 0 ? h.rows.length - 1 : inspection,
    ),
  );
  const row = h.rows[index];
  useEffect(() => {
    setInspection(-1);
    setData(null);
    setSources(null);
    setDataOpen(false);
    setSourcesOpen(false);
    dataGeneration.current++;
    sourceGeneration.current++;
  }, [h.header?.id]);
  useEffect(
    () => () => {
      dataGeneration.current++;
      sourceGeneration.current++;
    },
    [],
  );
  async function loadData(offset: number) {
    const origin = ++dataGeneration.current;
    setData(null);
    setDataError("Loading daily data…");
    try {
      const p = await h.dataPage(offset);
      if (origin === dataGeneration.current) {
        setData(p);
        setDataError("");
      }
    } catch (e) {
      if (origin === dataGeneration.current) setDataError(errorText(e));
    }
  }
  async function loadSources(offset: number) {
    const origin = ++sourceGeneration.current;
    setSources(null);
    setSourceError("Loading sources…");
    try {
      const p = await h.evidencePage(offset);
      if (origin === sourceGeneration.current) {
        setSources(p);
        setSourceError("");
      }
    } catch (e) {
      if (origin === sourceGeneration.current) setSourceError(errorText(e));
    }
  }
  const exact = (value: string | null, unit: string) =>
    value === null ? "Unavailable" : `${value} ${unit}`;
  return (
    <Card className="portfolio-card portfolio-performance">
      <div className="portfolio-cardhead">
        <h2>Performance</h2>
        <div className="portfolio-segmented">
          {(
            [
              ["return", "Return %"],
              ["value", "Value $"],
            ] as const
          ).map(([value, label]) => (
            <Button
              key={value}
              variant={mode === value ? "secondary" : "ghost"}
              aria-pressed={mode === value}
              onClick={() => setMode(value)}
            >
              {label}
            </Button>
          ))}
        </div>
      </div>
      <div className="portfolio-charttools">
        <div className="portfolio-legend">
          <span>● My portfolio</span>
          <span className="portfolio-benchmark">
            {mode === "return"
              ? "● S&P 500 total return"
              : "● Same cash flows in S&P 500"}
          </span>
        </div>
        <div className="portfolio-segmented">
          {(["1M", "3M", "YTD", "1Y", "All"] as const).map((period) => (
            <Button
              key={period}
              variant={period === h.period ? "secondary" : "ghost"}
              aria-pressed={period === h.period}
              onClick={() => h.setPeriod(period)}
            >
              {period}
            </Button>
          ))}
        </div>
      </div>
      <p role="status" className="portfolio-muted">
        {h.status}
      </p>
      <div className="portfolio-chart">
        {h.rows.length ? (
          <svg
            viewBox="0 0 1000 270"
            role="img"
            aria-label={`${mode === "return" ? "Return" : "Value"} comparison. Use left and right arrow keys to inspect dates.`}
            tabIndex={0}
            onKeyDown={(e) => {
              if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) {
                e.preventDefault();
                setInspection(
                  Math.max(
                    0,
                    Math.min(
                      h.rows.length - 1,
                      e.key === "Home"
                        ? 0
                        : e.key === "End"
                          ? h.rows.length - 1
                          : index + (e.key === "ArrowRight" ? 1 : -1),
                    ),
                  ),
                );
              }
            }}
            onPointerMove={(e) => {
              const rect = e.currentTarget.getBoundingClientRect();
              setInspection(
                Math.max(
                  0,
                  Math.min(
                    h.rows.length - 1,
                    Math.round(
                      ((((e.clientX - rect.left) / rect.width) * 1000 - 70) /
                        900) *
                        (h.rows.length - 1),
                    ),
                  ),
                ),
              );
            }}
          >
            {Array.from({ length: 5 }, (_, i) => {
              const y = 30 + i * 50;
              const value =
                geometry.max - ((geometry.max - geometry.min) * i) / 4;
              return (
                <g key={i}>
                  <line
                    x1="70"
                    x2="970"
                    y1={y}
                    y2={y}
                    className="portfolio-gridline"
                  />
                  <text
                    x="60"
                    y={y + 4}
                    textAnchor="end"
                    className="portfolio-axis"
                  >
                    {new Intl.NumberFormat("en-US", {
                      notation: "compact",
                      maximumFractionDigits: 1,
                    }).format(value)}
                    {mode === "return" ? "%" : ""}
                  </text>
                </g>
              );
            })}
            {geometry.benchmark.map((d, i) => (
              <path
                key={`b${i}`}
                d={d}
                fill="none"
                className="portfolio-benchmark-line"
              />
            ))}
            {geometry.portfolio.map((d, i) => (
              <path
                key={`p${i}`}
                d={d}
                fill="none"
                className="portfolio-series-line"
              />
            ))}
            {sampled.map((point, i) =>
              hasFlow(point) ? (
                <circle
                  key={point.date}
                  cx={geometry.points[i].x}
                  cy="238"
                  r="3"
                  className="portfolio-flow-marker"
                />
              ) : null,
            )}
            <text x="70" y="260" className="portfolio-axis">
              {h.rows[0].date}
            </text>
            <text x="970" y="260" textAnchor="end" className="portfolio-axis">
              {h.rows.at(-1)!.date}
            </text>
            <line
              x1={full.points[index]?.x ?? 70}
              x2={full.points[index]?.x ?? 70}
              y1="25"
              y2="235"
              className="portfolio-chart-cursor"
            />
          </svg>
        ) : (
          <p className="portfolio-empty-chart">
            Historical data will appear here.
          </p>
        )}
      </div>
      <output className="portfolio-inspection" aria-live="polite">
        {row &&
          `${row.date} · Portfolio ${mode === "return" ? formatPercent(portfolioPlotValue(row, mode), true) : formatMoney(portfolioPlotValue(row, mode))}${row.segment > 0 ? " (new return segment)" : ""} · S&P 500 ${mode === "return" ? formatPercent(row.benchmark_return_percent, true) : formatMoney(row.hypothetical_value)}${hasFlow(row) ? ` · Deposits ${formatMoney(row.deposits)}, opening contribution ${formatMoney(row.opening_contribution)}, withdrawals ${formatMoney(row.withdrawals)}` : ""}${row.issues.length ? " · " + [...new Set(row.issues.map((i) => i.code.replaceAll("_", " ")))].join(", ") : ""}`}
      </output>
      <p className="portfolio-muted">
        {mode === "return"
          ? "Daily time-weighted returns include dividends and fees. Deposits, withdrawals and opening balances are accounted for. Gaps indicate unavailable returns."
          : "Portfolio value includes cash. The benchmark shows a hypothetical investment with the same external cash flows; it stops if a withdrawal would exceed its balance."}
        {h.rows.some((r) => r.segment > 0)
          ? " New return segments restart at recapitalization; the full-period return is unavailable."
          : ""}
      </p>
      <div className="portfolio-actions">
        <Button
          variant="outline"
          disabled={!h.online || h.busy}
          title={h.online ? "" : "History refresh is unavailable offline"}
          onClick={() => void h.refresh()}
        >
          Refresh history
        </Button>
        {h.busy && (
          <Button variant="outline" onClick={() => void h.cancel()}>
            Cancel history
          </Button>
        )}
      </div>
      <details
        className="portfolio-history-data"
        open={dataOpen}
        onToggle={(e) => {
          const open = e.currentTarget.open;
          setDataOpen(open);
          if (open && !data && h.header) void loadData(0);
        }}
      >
        <summary>View exact daily data</summary>
        <p role="status">{dataError}</p>
        {data && (
          <>
            <Table>
              <TableHeader>
                <TableRow>
                  {[
                    "Date",
                    "Value",
                    "Portfolio return",
                    "S&P 500 return",
                    "Hypothetical value",
                    "Deposits",
                    "Opening contribution",
                    "Withdrawals",
                    "Coverage",
                  ].map((t) => (
                    <TableHead key={t}>{t}</TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.items.map((r) => (
                  <TableRow key={r.date}>
                    {[
                      r.date,
                      exact(r.value, "USD"),
                      exact(r.portfolio_return_percent, "%"),
                      exact(r.benchmark_return_percent, "%"),
                      exact(r.hypothetical_value, "USD"),
                      exact(r.deposits, "USD"),
                      exact(r.opening_contribution, "USD"),
                      exact(r.withdrawals, "USD"),
                      r.issues
                        .map((i) => i.code.replaceAll("_", " "))
                        .join("; ") || "Available",
                    ].map((v, i) => (
                      <TableCell key={i}>{v}</TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            <div className="portfolio-actions">
              {data.items[0]?.date !== h.rows[0]?.date && (
                <Button variant="outline" onClick={() => void loadData(0)}>
                  First dates
                </Button>
              )}
              {data.next_offset !== null && (
                <Button
                  variant="outline"
                  onClick={() => void loadData(data.next_offset!)}
                >
                  Next dates
                </Button>
              )}
            </div>
          </>
        )}
      </details>
      <details
        className="portfolio-history-methodology"
        open={sourcesOpen}
        onToggle={(e) => {
          const open = e.currentTarget.open;
          setSourcesOpen(open);
          if (open && !sources && h.header) void loadSources(0);
        }}
      >
        <summary>Methodology &amp; sources</summary>
        <p className="portfolio-muted">
          Return is compounded from daily growth: (closing value + withdrawals)
          ÷ (previous closing value + deposits + opening contribution). Inflows
          are treated as available for the day; withdrawals occur after daily
          growth. Existing accounts enter at market value on their setup date.
          The S&amp;P 500 total-return index (^SP500TR) already includes
          reinvested dividends.
        </p>
        <p role="status">{sourceError}</p>
        {sources?.items.map((s, i) => (
          <article key={i}>
            <strong>
              {s.manifest?.instrument.value ?? "Historical prices"} ·{" "}
              {s.provider.instance_id} ({s.provider.plugin_id}{" "}
              {s.provider.plugin_version})
            </strong>
            {s.manifest && (
              <p>
                Observed {s.manifest.coverage_start} through{" "}
                {s.manifest.last_completed_session}. Retrieved{" "}
                {s.manifest.retrieved_at}. Split normalization anchor{" "}
                {s.manifest.anchor}; {s.manifest.calendar} calendar{" "}
                {s.manifest.calendar_version}.
              </p>
            )}
            {safeUrl(s.source_url) && (
              <a
                href={safeUrl(s.source_url)!}
                target="_blank"
                rel="noopener noreferrer"
              >
                Source price history
              </a>
            )}
          </article>
        ))}
        {sources && !sources.items.length && (
          <p>No source price history was available for this result.</p>
        )}
        {sources?.next_offset != null && (
          <Button
            variant="outline"
            onClick={() => void loadSources(sources.next_offset!)}
          >
            More sources
          </Button>
        )}
      </details>
    </Card>
  );
}
function safeUrl(value: string | null | undefined) {
  try {
    const url = new URL(value ?? "");
    return ["https:", "http:"].includes(url.protocol) ? url.href : null;
  } catch {
    return null;
  }
}
