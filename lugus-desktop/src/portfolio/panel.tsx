import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Plus, RefreshCw } from "lucide-react";
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
import type { PortfolioApi } from "./api";
import type {
  Account,
  Audit,
  Header,
  Page,
  Receipt,
  TransactionRow,
  View,
  Instrument,
  ProviderIdentity,
} from "./types";
import { CommandDialog, Field, SelectField, errorText, today } from "./forms";
import {
  AccountDialog,
  BindingDialog,
  InstrumentDialog,
  type MutationProps,
} from "./accounts";
import { TransactionDialog, VoidDialog } from "./transactions";
import { Overview } from "./overview";
import { Holdings } from "./holdings";
import { HoldingDetail } from "./holding-detail";
import { usePortfolioHistory } from "./history-controller";
import { formatUsd } from "./format";
import {
  beginPortfolioLoad,
  failPortfolioLoad,
  finishPortfolioLoad,
  finishPortfolioRefresh,
  portfolioScope,
  selectedPortfolioLoad,
} from "./load-state";
type DialogState =
  | {
      kind: "create";
    }
  | {
      kind: "account";
      existing?: Account;
    }
  | {
      kind: "instrument";
    }
  | {
      kind: "rename";
      account: Account;
    }
  | {
      kind: "binding";
      instrument: Instrument;
      providers: {
        instance_id: string;
        name: string;
      }[];
    }
  | {
      kind: "benchmark";
      providers: ProviderIdentity[];
    }
  | {
      kind: "transaction";
      nextOrder: number;
      original?: TransactionRow;
    }
  | {
      kind: "void";
      row: TransactionRow;
    };
function NameDialog({
  title,
  label,
  initial,
  api,
  envelope,
  onClose,
  onSaved,
  mutation,
}: {
  title: string;
  label: string;
  initial: string;
  api: PortfolioApi;
  envelope: {
    portfolio_id: string | null;
    expected_revision: string;
  };
  onClose: () => void;
  onSaved: (r: Receipt) => Promise<void>;
  mutation: (name: string) => Record<string, unknown>;
}) {
  const [name, setName] = useState(initial);
  return (
    <CommandDialog
      {...{ title, api, envelope, onClose, onSaved }}
      mutation={() => mutation(name)}
    >
      <Field label={label} value={name} onChange={setName} />
    </CommandDialog>
  );
}
function BenchmarkDialog({
  providers,
  ...props
}: MutationProps & {
  providers: ProviderIdentity[];
}) {
  const [choice, setChoice] = useState(props.view.benchmark_instance_id ?? "");
  return (
    <CommandDialog
      title="S&P 500 total-return source"
      {...props}
      envelope={{
        portfolio_id: props.view.id,
        expected_revision: props.view.revision,
      }}
      mutation={() => ({
        kind: "set_benchmark_provider",
        instance_id: choice || null,
      })}
    >
      <SelectField
        label="Benchmark provider"
        value={choice}
        onChange={setChoice}
        options={[
          { value: "", label: "Automatic (one compatible provider)" },
          ...providers.map((p) => ({
            value: p.instance_id,
            label: `${p.instance_id} · ${p.plugin_id} ${p.plugin_version}`,
          })),
        ]}
      />
    </CommandDialog>
  );
}
export function PortfolioPanel({
  api,
  online,
  visible,
  onBack,
  onUse,
}: {
  api: PortfolioApi;
  online: boolean;
  visible: boolean;
  onBack: () => void;
  onUse: (
    view: View,
    accountId: string | null,
    intent?: {
      name: string;
      symbol: string;
    },
  ) => Promise<void>;
}) {
  const [portfolioId, setPortfolioId] = useState<string | null>(null);
  const [accountId, setAccountId] = useState<string | null>(null);
  const [section, setSection] = useState("overview");
  const [loadState, setLoadState] = useState(() =>
    beginPortfolioLoad(portfolioScope(null, null)),
  );
  const [feedback, setFeedback] = useState("");
  const [reload, setReload] = useState(0);
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [holding, setHolding] = useState<{
    id: string;
    trigger: HTMLElement;
  } | null>(null);
  const [transactions, setTransactions] = useState<TransactionRow[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [audits, setAudits] = useState<Audit[]>([]);
  const [auditNext, setAuditNext] = useState<number | null>(null);
  const [rowsLoading, setRowsLoading] = useState(false);
  const [refreshId, setRefreshId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const selectionKey = portfolioScope(portfolioId, accountId);
  const load = selectedPortfolioLoad(loadState, selectionKey);
  const { headers, whole, view } = load;
  const loading = load.status === "loading";
  const generation = useRef(0);
  const scope = useRef("");
  scope.current = JSON.stringify([visible, portfolioId, accountId, reload]);
  const history = usePortfolioHistory(
    api,
    view,
    accountId,
    online,
    visible && !loading,
  );
  async function pages<T>(
    v: View,
    kind: string,
    selectedAccount: string | null,
  ): Promise<T[]> {
    const output: T[] = [];
    let offset = 0;
    do {
      const page = await api<Page<T>>({
        kind: "rows",
        portfolio_id: v.id,
        account_id: selectedAccount,
        section: kind,
        offset,
        revision: v.revision,
      });
      if (page.revision !== v.revision)
        throw new Error("Portfolio changed. Reload this view.");
      output.push(...page.items);
      if (page.next_offset === null) return output;
      if (page.next_offset <= offset)
        throw new Error("Saved data pagination did not advance.");
      offset = page.next_offset;
    } while (true);
  }
  useEffect(() => {
    const origin = ++generation.current;
    setHolding(null);
    setDialog(null);
    if (!visible) return;
    setLoadState(beginPortfolioLoad(selectionKey));
    setFeedback("");
    let alive = true;
    void (async () => {
      try {
        const all: Header[] = [];
        let offset = 0;
        do {
          const p = await api<Page<Header>>({ kind: "list", offset });
          if (!alive) return;
          all.push(...p.items);
          if (p.next_offset === null) break;
          if (p.next_offset <= offset)
            throw new Error("Portfolio pagination did not advance.");
          offset = p.next_offset;
        } while (true);
        if (!alive) return;
        if (!all.length) {
          setLoadState((current) =>
            finishPortfolioLoad(current, selectionKey, all, null, null),
          );
          return;
        }
        const id =
          portfolioId && all.some((p) => p.id === portfolioId)
            ? portfolioId
            : all[0].id;
        if (id !== portfolioId) {
          setPortfolioId(id);
          return;
        }
        const total = await api<View>({
          kind: "overview",
          portfolio_id: id,
          account_id: null,
        });
        const scoped = accountId
          ? await api<View>({
              kind: "overview",
              portfolio_id: id,
              account_id: accountId,
            })
          : total;
        if (!alive || origin !== generation.current) return;
        setLoadState((current) =>
          finishPortfolioLoad(current, selectionKey, all, total, scoped),
        );
      } catch (e) {
        if (alive)
          setLoadState((current) =>
            failPortfolioLoad(current, selectionKey, errorText(e)),
          );
      }
    })();
    return () => {
      alive = false;
      generation.current++;
    };
  }, [api, visible, portfolioId, accountId, reload]);
  useEffect(() => {
    if (!visible || loading || !view) return;
    let alive = true;
    setRowsLoading(true);
    setTransactions([]);
    setAccounts([]);
    setAudits([]);
    setAuditNext(null);
    void (async () => {
      try {
        if (section === "transactions") {
          const r = await pages<TransactionRow>(
            view,
            "transactions",
            accountId,
          );
          if (alive) setTransactions(r);
        }
        if (section === "accounts") {
          const r = await pages<Account>(view, "accounts", accountId);
          if (alive) setAccounts(r);
        }
        if (section === "audit") {
          const p = await api<Page<Audit>>({
            kind: "audit",
            portfolio_id: view.id,
            offset: 0,
          });
          if (alive) {
            setAudits(p.items);
            setAuditNext(p.next_offset);
          }
        }
      } catch (e) {
        if (alive) setFeedback(errorText(e));
      } finally {
        if (alive) setRowsLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [api, visible, loading, view, accountId, section]);
  const saved = async (r: Receipt) => {
    generation.current++;
    setDialog(null);
    setPortfolioId(r.portfolio_id);
    setAccountId(null);
    setReload((v) => v + 1);
  };
  function retryLoad() {
    generation.current++;
    setReload((v) => v + 1);
  }
  async function act(action: () => Promise<void>) {
    const origin = scope.current;
    const originGeneration = generation.current;
    try {
      await action();
    } catch (e) {
      if (origin === scope.current && originGeneration === generation.current)
        setFeedback(errorText(e));
    }
  }
  async function addTransaction() {
    if (!whole || !view) return;
    if (!whole.accounts.length) throw new Error("Add an account first.");
    const origin = scope.current;
    const rows = await pages<TransactionRow>(view, "transactions", accountId);
    if (origin !== scope.current) return;
    const nextOrder = rows
      .filter((r) => r.event.date === today())
      .reduce((n, r) => Math.max(n, r.event.order + 1), 0);
    setDialog({ kind: "transaction", nextOrder });
  }
  async function refreshPrices() {
    if (!view || refreshing) return;
    const origin = scope.current;
    const originGeneration = generation.current;
    const expectedRevision = view.revision;
    const active = () =>
      origin === scope.current && originGeneration === generation.current;
    setRefreshing(true);
    setFeedback("Refreshing prices…");
    try {
      let result = await api<{ id: string; status: string }>({
        kind: "refresh",
        request: {
          request_id: crypto.randomUUID(),
          portfolio_id: view.id,
          expected_revision: expectedRevision,
        },
      });
      if (active()) setRefreshId(result.id);
      while (result.status === "running") {
        await new Promise((resolve) => setTimeout(resolve, 500));
        result = await api({ kind: "refresh_status", id: result.id });
      }
      if (!active()) return;
      setRefreshId(null);
      const total = await api<View>({
        kind: "overview",
        portfolio_id: view.id,
        account_id: null,
      });
      if (!active()) return;
      const scoped = accountId
        ? await api<View>({
            kind: "overview",
            portfolio_id: view.id,
            account_id: accountId,
          })
        : total;
      if (!active()) return;
      if (total.revision !== scoped.revision)
        throw new Error(
          "Portfolio changed while refreshing. Retry the refresh.",
        );
      await history.refresh(scoped);
      if (!active()) return;
      setLoadState((current) =>
        active()
          ? finishPortfolioRefresh(
              current,
              selectionKey,
              expectedRevision,
              total,
              scoped,
            )
          : current,
      );
      setFeedback(
        result.status === "partial"
          ? "Refresh finished with unpriced or unavailable observations. See price refresh details."
          : `Refresh: ${result.status}`,
      );
    } finally {
      setRefreshing(false);
      setRefreshId(null);
    }
  }
  async function moreAudit() {
    if (!view || auditNext === null) return;
    const origin = scope.current;
    const offset = auditNext;
    setRowsLoading(true);
    try {
      const p = await api<Page<Audit>>({
        kind: "audit",
        portfolio_id: view.id,
        offset,
      });
      if (origin !== scope.current) return;
      if (p.next_offset !== null && p.next_offset <= offset)
        throw new Error("Audit pagination did not advance.");
      setAudits((r) => [...r, ...p.items]);
      setAuditNext(p.next_offset);
    } finally {
      setRowsLoading(false);
    }
  }
  const openHolding = (id: string, trigger: HTMLElement) =>
    setHolding({ id, trigger });
  if (!visible) return <section id="portfolio" hidden />;
  const mutationProps = whole
    ? { api, view: whole, onClose: () => setDialog(null), onSaved: saved }
    : null;
  return (
    <section id="portfolio" className="portfolio-workspace portfolio-panel">
      <header className="portfolio-header">
        <div>
          <p className="eyebrow">Your investments</p>
          <h1>{view?.name ?? "Your portfolio"}</h1>
        </div>
        <Button variant="outline" onClick={onBack}>
          <ArrowLeft />
          Back to research
        </Button>
      </header>
      <p id="portfolio-feedback" role="status">
        {feedback}
      </p>
      {loading ? (
        <p>Loading portfolio…</p>
      ) : load.status === "error" ? (
        <Card className="portfolio-card">
          <h2>Unable to load portfolio</h2>
          <p role="alert">{load.error}</p>
          <Button variant="outline" onClick={retryLoad}>
            Retry
          </Button>
        </Card>
      ) : !headers.length ? (
        <Card className="portfolio-card">
          <h2>A clearer view of your investments</h2>
          <p>
            Track stocks, ETFs and cash in USD. Enter your transaction history
            or start with the holdings you already own.
          </p>
          <Button onClick={() => setDialog({ kind: "create" })}>
            <Plus />
            Create portfolio
          </Button>
        </Card>
      ) : view && whole ? (
        <>
          <div className="portfolio-actions">
            <SelectField
              label="Portfolio"
              value={portfolioId ?? ""}
              onChange={(id) => {
                setPortfolioId(id);
                setAccountId(null);
              }}
              options={headers.map((p) => ({ value: p.id, label: p.name }))}
            />
            <SelectField
              label="Account"
              value={accountId ?? ""}
              onChange={(id) => setAccountId(id || null)}
              options={[
                { value: "", label: "All accounts" },
                ...whole.accounts.map((a) => ({ value: a.id, label: a.name })),
              ]}
            />
            <Button
              variant="outline"
              onClick={() => setDialog({ kind: "create" })}
            >
              New portfolio
            </Button>
            <Button
              onClick={() => void act(addTransaction)}
              disabled={refreshing}
            >
              <Plus />
              Add transaction
            </Button>
            <Button
              variant="outline"
              onClick={() => void act(() => onUse(whole, accountId))}
            >
              Use in chat
            </Button>
            <Button
              variant="outline"
              onClick={() => void act(refreshPrices)}
              disabled={!online || refreshing}
            >
              <RefreshCw />
              Refresh prices
            </Button>
            {refreshId && (
              <Button
                variant="outline"
                onClick={() =>
                  void act(async () => {
                    await api({ kind: "refresh_cancel", id: refreshId });
                  })
                }
              >
                Cancel refresh
              </Button>
            )}
          </div>
          <nav className="portfolio-tabs" aria-label="Portfolio sections">
            {["overview", "holdings", "transactions", "accounts", "audit"].map(
              (name) => (
                <Button
                  key={name}
                  variant={section === name ? "secondary" : "ghost"}
                  aria-current={section === name ? "page" : undefined}
                  onClick={() => {
                    setSection(name);
                    setHolding(null);
                  }}
                >
                  {name[0].toUpperCase() + name.slice(1)}
                </Button>
              ),
            )}
          </nav>
          {section === "overview" && (
            <Overview view={view} history={history} onOpen={openHolding} />
          )}{" "}
          {section === "holdings" && (
            <Holdings view={view} onOpen={openHolding} />
          )}
          {section === "transactions" && (
            <Card className="portfolio-card">
              <h2>Transactions</h2>
              {rowsLoading ? (
                <p>Loading transactions…</p>
              ) : !transactions.length ? (
                <p>
                  No transactions yet. Add a deposit before recording purchases
                  in an account with full history.
                </p>
              ) : (
                <Table>
                  <TableHeader>
                    <TableRow>
                      {[
                        "Date / order",
                        "Account",
                        "Activity",
                        "Details",
                        "Actions",
                      ].map((label) => (
                        <TableHead key={label}>{label}</TableHead>
                      ))}
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {transactions.map((row) => {
                      const e = row.event;
                      const instrument = view.instruments.find(
                        (i) => i.id === e.kind.instrument_id,
                      );
                      return (
                        <TableRow key={`${row.account_id}-${e.id}`}>
                          <TableCell>
                            {e.date} / {e.order}
                          </TableCell>
                          <TableCell>{row.account_name}</TableCell>
                          <TableCell>{e.kind.kind}</TableCell>
                          <TableCell>
                            {e.kind.quantity
                              ? `${e.kind.quantity} ${instrument?.symbol ?? ""} · gross ${formatUsd(String(e.kind.gross))} · fees ${formatUsd(String(e.kind.fees))}`
                              : e.kind.amount
                                ? formatUsd(String(e.kind.amount))
                                : `${e.kind.numerator}:${e.kind.denominator} ${instrument?.symbol ?? ""}`}
                          </TableCell>
                          <TableCell>
                            <Button
                              variant="ghost"
                              onClick={() =>
                                setDialog({
                                  kind: "transaction",
                                  nextOrder: e.order,
                                  original: row,
                                })
                              }
                            >
                              Correct
                            </Button>
                            <Button
                              variant="ghost"
                              onClick={() => setDialog({ kind: "void", row })}
                            >
                              Void
                            </Button>
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              )}
            </Card>
          )}
          {section === "accounts" && (
            <>
              <div className="portfolio-actions">
                <Button
                  variant="outline"
                  onClick={() =>
                    void act(async () => {
                      const origin = scope.current;
                      const providers = await api<ProviderIdentity[]>({
                        kind: "history_providers",
                      });
                      if (origin === scope.current)
                        setDialog({ kind: "benchmark", providers });
                    })
                  }
                >
                  Choose benchmark source
                </Button>
                <Button onClick={() => setDialog({ kind: "account" })}>
                  Add account
                </Button>
                <Button
                  variant="outline"
                  onClick={() => setDialog({ kind: "instrument" })}
                >
                  Add instrument
                </Button>
              </div>
              {rowsLoading ? (
                <p>Loading accounts…</p>
              ) : (
                accounts.map((a) => (
                  <Card key={a.id} className="portfolio-account portfolio-card">
                    <h3>{a.name}</h3>
                    <p>USD · starts {a.start}</p>
                    <div className="portfolio-actions">
                      <Button
                        variant="outline"
                        onClick={() =>
                          setDialog({ kind: "account", existing: a })
                        }
                      >
                        Correct starting balances
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() =>
                          setDialog({ kind: "rename", account: a })
                        }
                      >
                        Rename
                      </Button>
                    </div>
                  </Card>
                ))
              )}
              <h2>Instruments</h2>
              {whole.instruments.map((i) => (
                <Card key={i.id} className="portfolio-account portfolio-card">
                  <h3>
                    {i.symbol} — {i.name}
                  </h3>
                  <p>
                    {i.asset_kind.toUpperCase()} · USD ·{" "}
                    {i.binding ? "Price source connected" : "No price source"}
                  </p>
                  <Button
                    variant="outline"
                    onClick={() =>
                      void act(async () => {
                        const origin = scope.current;
                        const providers = await api<
                          {
                            instance_id: string;
                            name: string;
                          }[]
                        >({ kind: "providers" });
                        if (origin !== scope.current) return;
                        if (!providers.length)
                          throw new Error(
                            "No market-data provider is configured. Configure a provider in Lugus to refresh prices.",
                          );
                        setDialog({
                          kind: "binding",
                          instrument: i,
                          providers,
                        });
                      })
                    }
                  >
                    Choose price source
                  </Button>
                </Card>
              ))}
            </>
          )}
          {section === "audit" && (
            <Card className="portfolio-card">
              <h2>Audit history</h2>
              {audits.map((row) => (
                <details key={row.request_id}>
                  <summary>
                    {row.recorded_at} ·{" "}
                    {String(row.mutation.kind).replaceAll("_", " ")} · revision{" "}
                    {row.revision}
                  </summary>
                  <pre>{JSON.stringify(row.mutation, null, 2)}</pre>
                </details>
              ))}
              {rowsLoading && <p>Loading audit history…</p>}
              {auditNext !== null && (
                <Button
                  variant="outline"
                  disabled={rowsLoading}
                  onClick={() => void act(moreAudit)}
                >
                  Older activity
                </Button>
              )}
            </Card>
          )}
        </>
      ) : (
        <Button variant="outline" onClick={retryLoad}>
          Retry
        </Button>
      )}
      {dialog?.kind === "create" && (
        <NameDialog
          title="Create portfolio"
          label="Portfolio name"
          initial="My portfolio"
          api={api}
          envelope={{ portfolio_id: null, expected_revision: "0" }}
          onClose={() => setDialog(null)}
          onSaved={saved}
          mutation={(name) => ({ kind: "create_portfolio", name })}
        />
      )}
      {mutationProps && dialog?.kind === "account" && (
        <AccountDialog {...mutationProps} existing={dialog.existing} />
      )}
      {mutationProps && dialog?.kind === "instrument" && (
        <InstrumentDialog {...mutationProps} />
      )}
      {mutationProps && dialog?.kind === "binding" && (
        <BindingDialog
          {...mutationProps}
          instrument={dialog.instrument}
          providers={dialog.providers}
        />
      )}
      {mutationProps && dialog?.kind === "benchmark" && (
        <BenchmarkDialog {...mutationProps} providers={dialog.providers} />
      )}
      {mutationProps && dialog?.kind === "transaction" && (
        <TransactionDialog
          {...mutationProps}
          accountId={accountId}
          nextOrder={dialog.nextOrder}
          original={dialog.original}
        />
      )}
      {mutationProps && dialog?.kind === "void" && (
        <VoidDialog {...mutationProps} row={dialog.row} />
      )}
      {whole && dialog?.kind === "rename" && (
        <NameDialog
          title="Rename account"
          label="Name"
          initial={dialog.account.name}
          api={api}
          envelope={{
            portfolio_id: whole.id,
            expected_revision: whole.revision,
          }}
          onClose={() => setDialog(null)}
          onSaved={saved}
          mutation={(name) => ({
            kind: "rename_account",
            account_id: dialog.account.id,
            name,
          })}
        />
      )}
      {holding && view && whole && (
        <HoldingDetail
          key={holding.id}
          api={api}
          view={view}
          accountId={accountId}
          id={holding.id}
          trigger={holding.trigger}
          onClose={() => setHolding(null)}
          onResearch={(intent) => onUse(whole, accountId, intent)}
        />
      )}
    </section>
  );
}
