import { useEffect, useRef, useState } from "react";
import { Button } from "../components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "../components/ui/sheet";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "../components/ui/table";
import type { PortfolioApi } from "./api";
import type { View, Page, LotRow, TransactionRow } from "./types";
import { formatMoney, formatPercent } from "./format";
import { errorText } from "./forms";
export interface ResearchIntent {
  instrumentId: string;
  symbol: string;
  name: string;
}
function HoldingRows({
  api,
  view,
  accountId,
  id,
  section,
}: {
  api: PortfolioApi;
  view: View;
  accountId: string | null;
  id: string;
  section: "lots" | "transactions";
}) {
  const [page, setPage] = useState<Page<LotRow | TransactionRow> | null>(null);
  const [offset, setOffset] = useState(0);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    setPage(null);
    setError("");
    void api<Page<LotRow | TransactionRow>>({
      kind: "holding_rows",
      portfolio_id: view.id,
      account_id: accountId,
      instrument_id: id,
      section,
      offset,
      revision: view.revision,
    })
      .then((p) => {
        if (!alive) return;
        if (p.revision !== view.revision)
          throw new Error("Portfolio changed. Reopen this holding.");
        setPage(p);
      })
      .catch((e) => {
        if (alive) setError(errorText(e));
      });
    return () => {
      alive = false;
    };
  }, [api, view.id, view.revision, accountId, id, section, offset]);
  return (
    <section>
      <h3>{section === "lots" ? "Remaining lots" : "Recent activity"}</h3>
      {error ? (
        <p role="status">{error}</p>
      ) : !page ? (
        <p>Loading…</p>
      ) : (
        <>
          {!page.items.length ? (
            <p>
              {section === "lots"
                ? "No remaining lots."
                : "No recorded activity."}
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  {(section === "lots"
                    ? ["Account", "Acquired", "Shares", "Basis"]
                    : ["Date", "Account", "Activity", "Amount / shares"]
                  ).map((v) => (
                    <TableHead key={v}>{v}</TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {page.items.map((item, index) => (
                  <TableRow key={index}>
                    {("lot" in item
                      ? [
                          item.account_name,
                          item.lot.acquired,
                          item.lot.quantity,
                          formatMoney(item.lot.basis),
                        ]
                      : [
                          item.event.date,
                          item.account_name,
                          item.event.kind.kind,
                          item.event.kind.amount
                            ? formatMoney(String(item.event.kind.amount))
                            : String(
                                item.event.kind.quantity ??
                                  `${item.event.kind.numerator}:${item.event.kind.denominator}`,
                              ),
                        ]
                    ).map((v, i) => (
                      <TableCell key={i}>{v}</TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
          <div className="portfolio-actions">
            {offset > 0 && (
              <Button variant="outline" onClick={() => setOffset(0)}>
                First page
              </Button>
            )}
            {page.next_offset !== null && (
              <Button
                variant="outline"
                onClick={() => setOffset(page.next_offset!)}
              >
                Next page
              </Button>
            )}
          </div>
        </>
      )}
    </section>
  );
}
export function HoldingDetail({
  api,
  view,
  accountId,
  id,
  trigger,
  onClose,
  onResearch,
}: {
  api: PortfolioApi;
  view: View;
  accountId: string | null;
  id: string;
  trigger: HTMLElement;
  onClose: () => void;
  onResearch: (intent: ResearchIntent) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );
  const holding = view.valuation.holdings.find((h) => h.instrument_id === id);
  const instrument = view.instruments.find((i) => i.id === id);
  if (!holding || !instrument) return null;
  const gain =
    view.dashboard?.holdings.find((h) => h.instrument_id === id)
      ?.unrealized_percent ?? null;
  return (
    <Sheet
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <SheetContent
        className="portfolio-drawer"
        onCloseAutoFocus={(e) => {
          e.preventDefault();
          if (trigger.isConnected) trigger.focus();
        }}
      >
        <SheetHeader>
          <SheetTitle>{instrument.symbol}</SheetTitle>
          <SheetDescription>
            {instrument.name} ·{" "}
            {instrument.asset_kind === "stock" ? "Stock" : "ETF"} · USD
          </SheetDescription>
        </SheetHeader>
        <div className="portfolio-drawer-body">
          <Button variant="outline" onClick={onClose}>
            Close holding
          </Button>
          <p className="portfolio-number">
            {formatMoney(holding.market_value)}
          </p>
          <dl className="portfolio-detailgrid">
            {[
              ["Shares", holding.quantity],
              ["Cost basis", formatMoney(holding.basis)],
              ["Unrealized gain / loss", formatMoney(holding.unrealized)],
              ["Return on cost", formatPercent(gain, true)],
              ["Portfolio weight", formatPercent(holding.allocation_percent)],
              [
                "Price",
                holding.price
                  ? `${formatMoney(holding.price.close)} · ${holding.price.date}`
                  : "Unpriced",
              ],
            ].map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
          {holding.unpriced_reason && (
            <p className="portfolio-notice">{holding.unpriced_reason}</p>
          )}
          {holding.simplified && (
            <p className="portfolio-notice">
              Includes simplified opening purchase history.
            </p>
          )}
          <p role="status">{error}</p>
          <Button
            className="portfolio-research-button"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onResearch({
                  instrumentId: id,
                  symbol: instrument.symbol,
                  name: instrument.name,
                });
                if (alive.current) onClose();
              } catch (e) {
                if (alive.current) setError(errorText(e));
              } finally {
                if (alive.current) setBusy(false);
              }
            }}
          >
            Research {instrument.symbol}
          </Button>
          <p className="portfolio-muted">
            Adds this portfolio context and a draft question to chat. You review
            and send it.
          </p>
          {(["lots", "transactions"] as const).map((section) => (
            <HoldingRows
              key={section}
              {...{ api, view, accountId, id, section }}
            />
          ))}
        </div>
      </SheetContent>
    </Sheet>
  );
}
