import { CompanyWorkbench } from "./companies/workbench";
import { companyApi } from "./companies/api";
import type { Company, CompanySummary } from "./companies/types";
import type { Message, Page } from "./types";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUp,
  BookOpen,
  BriefcaseBusiness,
  ChevronRight,
  Menu,
  MessageSquare,
  PanelRightClose,
  PanelRightOpen,
  Plus,
  Settings,
  Square,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
  SheetTrigger,
} from "@/components/ui/sheet";
import { ResearchPanel, viewLabel, type ResearchTab } from "./research/panel";
import { SettingsDialog } from "./settings-dialog";
import { PortfolioPanel } from "./portfolio/panel";
import { createPortfolioApi, type Rpc } from "./portfolio/api";
import { ChatController, terminal } from "./chat/controller";
import { MessageText } from "./chat/messages";
const rpc: Rpc = <T,>(request: object) => {
  const { operation, ...args } = request as {
    operation: string;
  };
  return invoke<T>("research", {
    payload: JSON.stringify({ op: operation, ...args }),
  });
};
const suggestions = [
  "Help me understand Apple’s business",
  "Explore Microsoft’s financial performance",
  "What should I look for in an annual report?",
];
export function App() {
  const [controller] = useState(() => new ChatController(rpc));
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const api = useMemo(() => createPortfolioApi(rpc), []);
  const [portfolio, setPortfolio] = useState(false);
  const [companiesOpen, setCompaniesOpen] = useState(true);
  const [companies, setCompanies] = useState<CompanySummary[]>([]);
  const [companyError, setCompanyError] = useState("");
  const [finding, setFinding] = useState<{
    company: CompanySummary;
    message: Message;
    text: string;
  } | null>(null);
  const [savingFinding, setSavingFinding] = useState(false);
  const [findingError, setFindingError] = useState("");
  const companyRpc = useMemo(() => companyApi(rpc), []);
  const changeCompany = (value: CompanySummary) => {
    setCompanies((previous) =>
      previous.some((c) => c.id === value.id)
        ? previous.map((c) => (c.id === value.id ? value : c))
        : [...previous, value],
    );
    controller.updateCompany(value);
  };
  useEffect(() => {
    let alive = true;
    const load = async () => {
      const items: CompanySummary[] = [];
      let offset: number | null = 0;
      while (offset !== null) {
        const page: Page<CompanySummary> = await companyRpc({
          kind: "list",
          offset,
        });
        items.push(...page.items);
        if (page.next_offset !== null && page.next_offset <= offset)
          throw new Error("Company list pagination did not advance.");
        offset = page.next_offset;
      }
      if (alive)
        setCompanies((previous) => [
          ...items,
          ...previous.filter((c) => !items.some((i) => i.id === c.id)),
        ]);
    };
    void load().catch((e) => {
      if (alive) setCompanyError(e?.message ?? String(e));
    });
    return () => {
      alive = false;
    };
  }, [companyRpc]);
  useEffect(() => {
    const company = companies.find(
      (c) => c.conversation_id === state.selection.id,
    );
    if (company && !state.company) controller.updateCompanySelection(company);
  }, [companies, state.selection.id, state.company, controller]);
  const openCompany = (company: CompanySummary) => {
    void controller.openCompany(company);
    setCompaniesOpen(true);
    setPortfolio(false);
    setNavigation(false);
  };
  const [settings, setSettings] = useState(false);
  const [inspection, setInspection] = useState<{
    viewId: string;
    tab: ResearchTab;
    sequence: number;
  } | null>(null);
  const [navigation, setNavigation] = useState(false);
  const [researchOpen, setResearchOpen] = useState(
    () => typeof window === "undefined" || window.innerWidth > 900,
  );
  const [width, setWidth] = useState(() => {
    try {
      return Math.max(
        320,
        Math.min(
          640,
          Number(localStorage.getItem("lugus:research-width")) || 410,
        ),
      );
    } catch {
      return 410;
    }
  });
  const input = useRef<HTMLTextAreaElement>(null);
  const messages = useRef<HTMLDivElement>(null);
  const nearBottom = useRef(true);
  useEffect(() => {
    void controller.initialize();
    return () => controller.dispose();
  }, [controller]);
  useLayoutEffect(() => {
    if (nearBottom.current && messages.current)
      messages.current.scrollTop = messages.current.scrollHeight;
  }, [
    state.messages,
    state.stream,
    state.activity,
    state.selection.id,
    portfolio,
    researchOpen,
  ]);
  const active = !!state.run && !terminal(state.run);
  const title =
    state.chats.find((chat) => chat.id === state.selection.id)?.title ??
    "New conversation";
  const resize = (value: number) => {
    const bounded = Math.max(
      320,
      Math.min(640, value, window.innerWidth - 580),
    );
    setWidth(bounded);
    try {
      localStorage.setItem("lugus:research-width", String(bounded));
    } catch {}
  };
  const focusComposer = () =>
    requestAnimationFrame(() => input.current?.focus());
  const showChat = () => {
    setPortfolio(false);
    setCompaniesOpen(false);
    setNavigation(false);
  };
  const evidenceLabel = (id: string) => {
    const item = state.research.find((item) => item.view.id === id);
    return item
      ? viewLabel(item, state.research)
      : state.receipts.find((view) => view.id === id)?.kind === "document"
        ? "Saved filing document"
        : "Unavailable research";
  };
  const revealResearch = () => {
    const current = controller.getSnapshot();
    const viewId = current.workspace?.selected_view_id;
    const item = current.research.find((item) => item.view.id === viewId);
    if (viewId)
      setInspection((previous) => ({
        viewId,
        tab:
          item?.data.header.kind === "facts"
            ? "financials"
            : item?.data.header.kind === "filings"
              ? "filings"
              : "overview",
        sequence: (previous?.sequence ?? 0) + 1,
      }));
    setResearchOpen(true);
    requestAnimationFrame(() => {
      const heading = document.getElementById("company-name");
      heading?.focus({ preventScroll: true });
      document.getElementById("research")?.scrollTo({ top: 0 });
    });
  };
  const inspectEvidence = async (id: string) => {
    const conversation = state.selection.id;
    if (id !== state.workspace?.selected_view_id)
      await controller.selectView(id);
    const current = controller.getSnapshot();
    if (
      current.selection.id === conversation &&
      current.workspace?.selected_view_id === id
    )
      revealResearch();
  };
  const returnToQuestion = () => {
    if (window.innerWidth <= 900) setResearchOpen(false);
    focusComposer();
  };
  const nav = (
    <>
      <div className="sidebar-brand">
        <span className="brand-mark">L</span>
        <span>
          Lugus<span className="brand-subtitle">A clearer view.</span>
        </span>
      </div>
      <Button
        id="new-chat"
        className="new-chat-button"
        onClick={() => {
          controller.newChat();
          showChat();
          setResearchOpen(window.innerWidth > 900);
          focusComposer();
        }}
      >
        <Plus size={16} />
        New chat
      </Button>
      <nav className="workspace-navigation" aria-label="Workspaces">
        <Button
          variant={companiesOpen && !portfolio ? "secondary" : "ghost"}
          aria-current={companiesOpen && !portfolio ? "page" : undefined}
          onClick={() => {
            setCompaniesOpen(true);
            setPortfolio(false);
            setNavigation(false);
          }}
        >
          <BookOpen size={17} />
          Companies
        </Button>
        <Button
          variant={!portfolio && !companiesOpen ? "secondary" : "ghost"}
          aria-current={!portfolio && !companiesOpen ? "page" : undefined}
          onClick={showChat}
        >
          <BookOpen size={17} />
          Research
        </Button>
        <Button
          id="open-portfolio"
          variant={portfolio ? "secondary" : "ghost"}
          aria-current={portfolio ? "page" : undefined}
          onClick={() => {
            setPortfolio(true);
            setNavigation(false);
          }}
        >
          <BriefcaseBusiness size={17} />
          Portfolio
        </Button>
      </nav>
      <div className="sidebar-section-title">Companies</div>
      <nav className="company-navigation" aria-label="Companies">
        {companies.map((company) => (
          <Button
            key={company.id}
            variant="ghost"
            aria-current={state.company?.id === company.id ? "page" : undefined}
            onClick={() => openCompany(company)}
          >
            <BookOpen size={15} />
            <span>{company.name}</span>
          </Button>
        ))}
      </nav>
      <div className="sidebar-section-title">Recent conversations</div>
      <nav id="chats" className="chat-navigation" aria-label="Conversations">
        {state.chats.map((chat) => (
          <Button
            key={chat.id}
            variant="ghost"
            aria-current={chat.id === state.selection.id ? "page" : undefined}
            title={chat.title}
            onClick={() => {
              nearBottom.current = true;
              const company = companies.find(
                (c) => c.conversation_id === chat.id,
              );
              void (company
                ? controller.openCompany(company)
                : controller.openChat(chat));
              showChat();
            }}
          >
            <MessageSquare size={15} />
            <span>{chat.title}</span>
          </Button>
        ))}
      </nav>
      {state.nextChats !== null && (
        <Button
          id="more-chats"
          variant="ghost"
          onClick={() =>
            void controller.loadChats(true).catch(controller.setError)
          }
        >
          More conversations
        </Button>
      )}
      <div className="sidebar-footer">
        <div id="connection" className="connection-status">
          <span
            className={
              state.runtimeAvailable ? "status-dot ready" : "status-dot"
            }
          />
          {state.connection}
        </div>
        <Button
          id="settings-button"
          variant="ghost"
          onClick={() => {
            setNavigation(false);
            setSettings(true);
          }}
        >
          <Settings size={16} />
          Settings
        </Button>
      </div>
    </>
  );
  return (
    <div className="app-shell">
      <aside className="sidebar">{!navigation && nav}</aside>
      <main className="main-workspace">
        <header className="workspace-header">
          <Sheet open={navigation} onOpenChange={setNavigation}>
            <SheetTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="mobile-navigation"
                aria-label="Open navigation"
              >
                <Menu />
              </Button>
            </SheetTrigger>
            <SheetContent side="left" className="navigation-sheet">
              <SheetHeader className="sr-only">
                <SheetTitle>Navigation</SheetTitle>
                <SheetDescription>
                  Workspaces and recent conversations
                </SheetDescription>
              </SheetHeader>
              {navigation && nav}
            </SheetContent>
          </Sheet>
          <div className="workspace-heading">
            <span className="eyebrow">
              {portfolio
                ? "Your investments"
                : companiesOpen
                  ? "Company workbench"
                  : state.company
                    ? `${state.company.name} · Research`
                    : "Research workspace"}
            </span>
            <h1 id="chat-title">
              {portfolio ? "Portfolio" : companiesOpen ? "Companies" : title}
            </h1>
          </div>
          {!portfolio && !companiesOpen && state.company && (
            <Button variant="outline" onClick={() => setCompaniesOpen(true)}>
              Company brief
            </Button>
          )}
          {!portfolio && !companiesOpen && (
            <Button
              id="toggle-research"
              variant="outline"
              aria-expanded={researchOpen}
              aria-controls="research"
              onClick={() => setResearchOpen((value) => !value)}
            >
              {researchOpen ? (
                <PanelRightClose size={16} />
              ) : (
                <PanelRightOpen size={16} />
              )}
              <span>{researchOpen ? "Hide research" : "Show research"}</span>
            </Button>
          )}
        </header>
        <div
          hidden={portfolio || !companiesOpen}
          className="company-workbench-container"
        >
          {companyError && (
            <p className="error-notice" role="alert">
              {companyError}
            </p>
          )}
          <CompanyWorkbench
            rpc={rpc}
            selected={state.company}
            companies={companies}
            onOpen={openCompany}
            onChange={changeCompany}
            onResearch={(messageId) => {
              showChat();
              if (messageId)
                requestAnimationFrame(() =>
                  document
                    .getElementById(`message-${messageId}`)
                    ?.scrollIntoView({ block: "center", behavior: "smooth" }),
                );
            }}
            onReview={() => {
              controller.prepareReview();
              showChat();
              focusComposer();
            }}
            refresh={`${state.run?.id}:${state.run?.status}`}
          />
        </div>
        <div
          hidden={portfolio || companiesOpen}
          className="research-workspace"
          data-research-open={researchOpen}
          style={{ "--research-width": `${width}px` } as React.CSSProperties}
        >
          <section className="chat-pane" aria-label="Conversation">
            <div
              id="messages"
              className="messages"
              ref={messages}
              onScroll={() => {
                const node = messages.current;
                if (node)
                  nearBottom.current =
                    node.scrollHeight - node.scrollTop - node.clientHeight <
                    100;
              }}
            >
              {state.loading ? (
                <p className="evidence-note" role="status">
                  Opening conversation…
                </p>
              ) : !state.messages.length && !state.run ? (
                <div className="chat-empty">
                  <div className="empty-brand">L</div>
                  <Badge variant="secondary">
                    Thoughtful research. Clearer decisions.
                  </Badge>
                  <h2>
                    What would you like
                    <br />
                    to understand?
                  </h2>
                  <p>
                    Explore companies, follow the evidence, and see the bigger
                    picture.
                  </p>
                  <div className="suggestions">
                    {suggestions.map((prompt) => (
                      <Button
                        key={prompt}
                        variant="outline"
                        onClick={() => {
                          controller.setDraft(prompt);
                          focusComposer();
                        }}
                      >
                        {prompt}
                        <ChevronRight size={16} />
                      </Button>
                    ))}
                  </div>
                </div>
              ) : (
                <>
                  {state.messages.map((message) => (
                    <article
                      className={`message ${message.role}`}
                      key={message.id}
                      id={`message-${message.id}`}
                    >
                      {message.role === "assistant" && (
                        <div className="assistant-mark" aria-hidden="true">
                          L
                        </div>
                      )}
                      <div className="message-body">
                        {message.role === "assistant" ? (
                          <>
                            <MessageText text={message.text} />
                            {state.company && (
                              <Button
                                variant="ghost"
                                size="sm"
                                className="save-finding"
                                onClick={() => {
                                  setFindingError("");
                                  setFinding({
                                    company: state.company!,
                                    message,
                                    text: Array.from(message.text)

                                      .join(""),
                                  });
                                }}
                              >
                                Save finding
                              </Button>
                            )}
                          </>
                        ) : (
                          message.text
                        )}
                      </div>
                    </article>
                  ))}
                  {active && (
                    <>
                      {state.stream && (
                        <article className="message assistant streaming">
                          <div className="assistant-mark" aria-hidden="true">
                            L
                          </div>
                          <div className="message-body">
                            <MessageText text={state.stream} />
                          </div>
                        </article>
                      )}
                      <div className="activity" role="status">
                        <span className="activity-dot" />
                        {state.activity || "Researching your question…"}
                      </div>
                    </>
                  )}
                </>
              )}
            </div>
            <div className="composer-area">
              {state.receipts.length > 0 && (
                <nav
                  className="conversation-evidence"
                  aria-label="Conversation evidence"
                >
                  <div className="evidence-heading">
                    <BookOpen size={14} aria-hidden="true" />
                    <span>Saved in this conversation</span>
                    <span className="evidence-count">
                      {state.receipts.length}
                    </span>
                  </div>
                  <div className="evidence-buttons">
                    {state.receipts.map((view) => (
                      <Button
                        key={view.id}
                        variant="outline"
                        size="sm"
                        aria-current={
                          view.id === state.workspace?.selected_view_id
                            ? "true"
                            : undefined
                        }
                        onClick={() => void inspectEvidence(view.id)}
                      >
                        {evidenceLabel(view.id)}
                        <ChevronRight size={14} aria-hidden="true" />
                      </Button>
                    ))}
                  </div>
                </nav>
              )}
              {state.error && (
                <p id="error" className="error-notice" role="alert">
                  {state.error}
                </p>
              )}
              <form
                id="composer"
                className="composer-card"
                onSubmit={(event) => {
                  event.preventDefault();
                  nearBottom.current = true;
                  void controller.send();
                }}
              >
                {state.company && (
                  <div className="context-chip company-context">
                    <BookOpen size={14} />
                    <button
                      type="button"
                      className="context-inspect"
                      onClick={() => setCompaniesOpen(true)}
                    >
                      <span className="context-caption">
                        Using saved company brief
                      </span>
                      <span>
                        {state.company.name} · revision {state.company.revision}
                      </span>
                    </button>
                  </div>
                )}
                {state.review && (
                  <div className="context-chip">
                    <span>
                      Thesis review · sending may retrieve fresh evidence
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      type="button"
                      onClick={controller.cancelReview}
                    >
                      Send as ordinary question
                    </Button>
                  </div>
                )}
                {state.workspace?.selected_view_id && state.includeContext && (
                  <div id="selected-context" className="context-chip">
                    <BookOpen size={14} />
                    <button
                      className="context-inspect"
                      type="button"
                      onClick={revealResearch}
                      title="Inspect the research included with your next message"
                    >
                      <span className="context-caption">Using as context</span>
                      <span>
                        {evidenceLabel(state.workspace.selected_view_id)}
                      </span>
                      <ChevronRight size={14} aria-hidden="true" />
                    </button>
                    <Button
                      variant="ghost"
                      size="icon"
                      type="button"
                      aria-label="Remove selected view from message context"
                      onClick={controller.removeContext}
                    >
                      <X size={14} />
                    </Button>
                  </div>
                )}
                {state.portfolioContext?.conversation ===
                  state.selection.id && (
                  <div id="portfolio-context" className="context-chip">
                    <span>
                      Portfolio snapshot as of {state.portfolioContext.date}{" "}
                      will be shared with the selected agent. Add a company hint
                      for company research.
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      type="button"
                      onClick={controller.removePortfolioContext}
                    >
                      Remove portfolio
                    </Button>
                  </div>
                )}
                <Textarea
                  id="message-input"
                  ref={input}
                  aria-label="Message Lugus"
                  placeholder="Ask a question about a company or your portfolio…"
                  value={state.draft}
                  onChange={(event) => controller.setDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (
                      event.key === "Enter" &&
                      !event.shiftKey &&
                      !event.nativeEvent.isComposing
                    ) {
                      event.preventDefault();
                      nearBottom.current = true;
                      void controller.send();
                    }
                  }}
                />
                <div className="composer-actions">
                  <div className="composer-meta">
                    <label htmlFor="company-hint">Company hint</label>
                    <Input
                      id="company-hint"
                      maxLength={256}
                      autoComplete="off"
                      placeholder="e.g. AAPL · optional"
                      disabled={!!state.company}
                      value={state.hint}
                      onChange={(event) =>
                        controller.setHint(event.target.value)
                      }
                    />
                  </div>
                  <span id="run-status" role="status">
                    {state.submitting
                      ? "Sending…"
                      : active
                        ? "Researching…"
                        : state.run?.status === "interrupted"
                          ? "Stopped"
                          : state.run?.status === "failed"
                            ? "Needs attention"
                            : ""}
                  </span>
                  {active ? (
                    <Button
                      id="stop"
                      type="button"
                      variant="outline"
                      onClick={() => void controller.stop()}
                    >
                      <Square size={14} />
                      Stop
                    </Button>
                  ) : (
                    <Button
                      id="send"
                      type="submit"
                      size="icon"
                      aria-label="Send message"
                      disabled={
                        !state.ready ||
                        !state.runtimeAvailable ||
                        state.submitting ||
                        !state.draft.trim()
                      }
                    >
                      <ArrowUp size={18} />
                    </Button>
                  )}
                </div>
              </form>
              <p className="composer-note">
                Research with sources. Use your judgment.
              </p>
            </div>
          </section>
          <div
            id="resize"
            className="research-resize"
            role="separator"
            tabIndex={researchOpen ? 0 : -1}
            aria-label="Research panel width"
            aria-orientation="vertical"
            aria-valuemin={320}
            aria-valuemax={640}
            aria-valuenow={width}
            onPointerDown={(event) =>
              event.currentTarget.setPointerCapture(event.pointerId)
            }
            onPointerMove={(event) => {
              if (event.currentTarget.hasPointerCapture(event.pointerId))
                resize(window.innerWidth - event.clientX);
            }}
            onKeyDown={(event) => {
              if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
                event.preventDefault();
                resize(width + (event.key === "ArrowLeft" ? 20 : -20));
              }
            }}
          />
          <aside
            id="research"
            className="research-pane"
            aria-label="Company research"
          >
            <ResearchPanel
              items={state.research}
              receipts={state.receipts}
              selectedId={state.workspace?.selected_view_id ?? null}
              issues={state.issues}
              onSelect={(id) => void controller.selectView(id)}
              onBackToQuestion={returnToQuestion}
              inspection={inspection}
            />
          </aside>
        </div>
        <PortfolioPanel
          api={api}
          online={state.online}
          visible={portfolio}
          onBack={showChat}
          onUse={async (view, accountId, intent) => {
            if (state.company) {
              const company =
                intent &&
                companies.find(
                  (c) => c.hint.toUpperCase() === intent.symbol.toUpperCase(),
                );
              if (company) await controller.openCompany(company);
              else controller.newChat();
            }
            await controller.usePortfolio(api, view, accountId, intent);
            setPortfolio(false);
            setCompaniesOpen(false);
            setResearchOpen(window.innerWidth > 900);
            focusComposer();
          }}
        />
      </main>
      <Dialog
        open={finding !== null}
        onOpenChange={(open) => {
          if (!open && !savingFinding) setFinding(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Accept research finding</DialogTitle>
            <DialogDescription>
              Edit the excerpt to save in {finding?.company.name}. The original
              answer remains linked; your thesis is unchanged.
            </DialogDescription>
          </DialogHeader>
          <Textarea
            aria-label="Finding text"
            value={finding?.text ?? ""}
            disabled={savingFinding}
            onChange={(e) =>
              setFinding((value) =>
                value ? { ...value, text: e.target.value } : null,
              )
            }
            rows={8}
          />
          {findingError && (
            <p role="alert" className="error-notice">
              {findingError}
            </p>
          )}
          <Button
            disabled={savingFinding || !finding?.text.trim()}
            onClick={async () => {
              if (!finding) return;
              setSavingFinding(true);
              setFindingError("");
              try {
                const saved = await companyRpc<Company>({
                  kind: "finding",
                  company: finding.company.id,
                  revision: finding.company.revision,
                  message: finding.message.id,
                  text: finding.text,
                });
                changeCompany(saved);
                setFinding(null);
              } catch (e) {
                setFindingError(
                  e && typeof e === "object" && "message" in e
                    ? String(e.message)
                    : String(e),
                );
              } finally {
                setSavingFinding(false);
              }
            }}
          >
            {savingFinding ? "Saving…" : "Accept finding"}
          </Button>
        </DialogContent>
      </Dialog>
      <SettingsDialog
        open={settings}
        onOpenChange={setSettings}
        rpc={rpc}
        onSaved={controller.applySettings}
      />
    </div>
  );
}
