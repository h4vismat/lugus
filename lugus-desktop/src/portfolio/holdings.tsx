import { useState } from "react";
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
import type { View } from "./types";
import { sortHoldings, type HoldingSort } from "./dashboard-state";
import { formatMoney, formatPercent } from "./format";
export function Holdings({
  view,
  onOpen,
}: {
  view: View;
  onOpen: (id: string, trigger: HTMLElement) => void;
}) {
  const [key, setKey] = useState<HoldingSort>("market_value");
  const [direction, setDirection] = useState<"asc" | "desc">("desc");
  return (
    <Card className="portfolio-card">
      <h2>Holdings</h2>
      {!view.valuation.holdings.length ? (
        <p>
          No open positions. Add a purchase or opening lots to see holdings.
        </p>
      ) : (
        <>
          <Table className="portfolio-holdings-table">
            <TableHeader>
              <TableRow>
                {(
                  [
                    ["symbol", "Holding"],
                    ["market_value", "Market value"],
                    ["allocation_percent", "Weight"],
                    ["basis", "Cost basis"],
                    ["unrealized", "Gain / loss"],
                    ["unrealized_percent", "Return"],
                  ] as const
                ).map(([field, label]) => (
                  <TableHead
                    key={field}
                    aria-sort={
                      key === field
                        ? direction === "asc"
                          ? "ascending"
                          : "descending"
                        : undefined
                    }
                  >
                    <Button
                      variant="ghost"
                      onClick={() => {
                        setDirection(
                          field === key && direction === "desc"
                            ? "asc"
                            : "desc",
                        );
                        setKey(field);
                      }}
                    >
                      {label}
                      {field === key ? (direction === "asc" ? " ↑" : " ↓") : ""}
                    </Button>
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {sortHoldings(view, key, direction).map((h) => {
                const i = view.instruments.find(
                  (i) => i.id === h.instrument_id,
                );
                const gain =
                  view.dashboard?.holdings.find(
                    (m) => m.instrument_id === h.instrument_id,
                  )?.unrealized_percent ?? null;
                return (
                  <TableRow key={h.instrument_id}>
                    <TableCell>
                      <Button
                        variant="link"
                        onClick={(e) =>
                          onOpen(h.instrument_id, e.currentTarget)
                        }
                      >
                        {i?.symbol ?? h.instrument_id}
                      </Button>
                      <small>{i?.name ?? ""}</small>
                    </TableCell>
                    {[
                      formatMoney(h.market_value),
                      formatPercent(h.allocation_percent),
                      formatMoney(h.basis),
                      formatMoney(h.unrealized),
                      formatPercent(gain, true),
                    ].map((v, i) => (
                      <TableCell
                        key={i}
                        className={
                          i >= 3 && h.unrealized !== null
                            ? h.unrealized.startsWith("-")
                              ? "portfolio-negative"
                              : "portfolio-positive"
                            : undefined
                        }
                      >
                        {v}
                      </TableCell>
                    ))}
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
          <p className="portfolio-muted">
            {view.valuation.complete
              ? "All holdings priced"
              : "Some holdings are unpriced"}{" "}
            · USD · Prices and lot details are available on each holding.
          </p>
        </>
      )}
    </Card>
  );
}
