import { useState } from "react";
import { Button } from "../components/ui/button";
import { Card } from "../components/ui/card";
import type { View } from "./types";
import { formatMoney, formatPercent, compareDecimal } from "./format";
export function Allocation({ view }: { view: View }) {
  const [mode, setMode] = useState<"holdings" | "types">("holdings");
  const holdings = view.valuation.holdings
    .map((h) => ({
      key: h.instrument_id,
      label:
        view.instruments.find((i) => i.id === h.instrument_id)?.symbol ??
        h.instrument_id,
      value: h.market_value,
      percent: h.allocation_percent,
    }))
    .sort((a, b) => compareDecimal(a.percent, b.percent, "desc"));
  holdings.push({
    key: "cash",
    label: "Cash",
    value: view.valuation.cash,
    percent: view.valuation.cash_allocation_percent,
  });
  const entries =
    mode === "holdings" ? holdings : (view.dashboard?.by_asset_type ?? []);
  const metric = view.dashboard;
  return (
    <div className="portfolio-below">
      <Card className="portfolio-card">
        <div className="portfolio-cardhead">
          <h2>Allocation</h2>
          <div className="portfolio-segmented">
            {(
              [
                ["holdings", "Holdings"],
                ["types", "Asset type"],
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
        {!view.valuation.complete ? (
          <p className="portfolio-muted">
            Allocation is unavailable until every holding has a compatible
            price.
          </p>
        ) : !entries.length || view.valuation.total_value === "0" ? (
          <p className="portfolio-muted">
            Fund an account to see your allocation.
          </p>
        ) : (
          entries.map((a, i) => (
            <div key={a.key} className="portfolio-barrow">
              <span title={formatMoney(a.value)}>{a.label}</span>
              <div className="portfolio-track">
                <div
                  className="portfolio-fill"
                  style={{
                    width: `${Math.min(100, Math.max(0, Number(a.percent ?? 0)))}%`,
                    opacity: Math.max(0.3, 1 - i * 0.11),
                  }}
                />
              </div>
              <span>{formatPercent(a.percent)}</span>
            </div>
          ))
        )}
      </Card>
      <Card className="portfolio-card">
        <h2>Concentration &amp; cash</h2>
        {metric?.largest_weight != null ? (
          <>
            <p className="portfolio-concentration">
              {formatPercent(metric.largest_weight)} in your largest holding
            </p>
            <p className="portfolio-muted">
              {view.instruments.find(
                (i) => i.id === metric.largest_instrument_id,
              )?.name ?? ""}
            </p>
            <p className="portfolio-insight">
              Your two largest positions make up{" "}
              {formatPercent(metric.top_two_weight)} of the portfolio.
            </p>
          </>
        ) : (
          <p className="portfolio-muted">
            {view.valuation.complete
              ? "No open security positions."
              : "Concentration is unavailable while holdings are unpriced."}
          </p>
        )}
        <p className="portfolio-insight">
          {formatMoney(view.valuation.cash)} cash ·{" "}
          {formatPercent(view.valuation.cash_allocation_percent)}
        </p>
        <p className="portfolio-muted">
          Cash is included in portfolio performance.
        </p>
      </Card>
    </div>
  );
}
