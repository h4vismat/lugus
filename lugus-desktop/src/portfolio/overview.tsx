import { Card } from "../components/ui/card";
import type { View } from "./types";
import type { HistoryController } from "./history-controller";
import {
  formatUsd,
  formatMoney,
  formatPercent,
  overviewLabels,
} from "./format";
import { Allocation } from "./allocation";
import { Holdings } from "./holdings";
import { Performance } from "./performance";
export function Overview({
  view,
  history,
  onOpen,
}: {
  view: View;
  history: HistoryController;
  onOpen: (id: string, trigger: HTMLElement) => void;
}) {
  const labels = overviewLabels(view);
  const summary = history.header?.summary;
  const stale = history.header && history.header.key.revision !== view.revision;
  return (
    <>
      <section className="portfolio-summary" aria-label="Portfolio summary">
        <article>
          <Card>
            <h3>{labels.valueLabel}</h3>
            <p className="portfolio-number">{labels.value}</p>
            <p className="portfolio-muted">
              {formatMoney(view.dashboard?.invested_market_value ?? null)}{" "}
              invested · {formatUsd(view.valuation.cash)} cash
            </p>
          </Card>
        </article>
        {[
          [
            "Portfolio return",
            "Cash flows accounted for",
            summary?.portfolio_return_percent ?? null,
          ],
          [
            "S&P 500",
            "Total return · dividends reinvested",
            summary?.benchmark_return_percent ?? null,
          ],
          [
            "Difference",
            "Percentage points vs. benchmark",
            summary?.difference_pp ?? null,
          ],
        ].map(([label, note, value], i) => (
          <article key={label}>
            <Card>
              <h3>
                {label}
                {i < 2 ? ` · ${history.period}` : ""}
                {stale ? " · saved" : ""}
              </h3>
              <p
                className={`portfolio-number ${value?.startsWith("-") ? "portfolio-negative" : i === 1 ? "" : "portfolio-positive"}`}
              >
                {i === 2
                  ? formatPercent(value, true).replace("%", " pp")
                  : formatPercent(value, true)}
              </p>
              <p className="portfolio-muted">{note}</p>
            </Card>
          </article>
        ))}
      </section>
      {!view.valuation.complete && (
        <p className="portfolio-notice">
          Some holdings are unpriced. The priced subtotal excludes them; total
          value and portfolio allocation are unavailable.
        </p>
      )}
      <Performance history={history} />
      <Holdings view={view} onOpen={onOpen} />
      <Allocation view={view} />
      <details className="portfolio-accounting">
        <summary>Accounting &amp; price details</summary>
        <div className="portfolio-metrics">
          {[
            ["Realized P&L", formatUsd(view.realized)],
            ["Unrealized P&L", formatMoney(view.valuation.unrealized)],
            ["Dividend income", formatUsd(view.dividends)],
            ["Standalone fees", formatUsd(view.standalone_fees)],
            ["Trade fees (included in P&L)", formatUsd(view.trade_fees)],
            ["Deposits", formatUsd(view.deposits)],
            ["Withdrawals", formatUsd(view.withdrawals)],
          ].map(([label, value]) => (
            <article key={label}>
              <h3>{label}</h3>
              <p>{value}</p>
            </article>
          ))}
        </div>
        <p>
          Accounting as of {view.as_of}. Sales use FIFO within each account.
        </p>
        {view.accounts.map((a) => (
          <p key={a.id}>
            {a.name}: recorded activity starts {a.start}.
            {a.simplified
              ? " Opening lots include simplified purchase history."
              : ""}
          </p>
        ))}
        {view.price_status.map((raw, index) => (
          <p key={index}>
            {view.instruments.reduce(
              (text, i) => text.replaceAll(i.id, i.symbol),
              raw,
            )}
          </p>
        ))}
      </details>
    </>
  );
}
