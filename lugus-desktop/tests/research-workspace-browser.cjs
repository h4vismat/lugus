// Exercises evidence navigation through the real app with a synthetic Tauri boundary.
const assert = require("node:assert/strict");
const { chromium } = require(
  process.env.LUGUS_PLAYWRIGHT_MODULE || "playwright",
);
const { spawn } = require("node:child_process");
const path = require("node:path");
const server = spawn(
  process.execPath,
  [
    path.resolve(__dirname, "../node_modules/vite/bin/vite.js"),
    "--host",
    "127.0.0.1",
    "--port",
    "4198",
    "--strictPort",
  ],
  { cwd: path.resolve(__dirname, ".."), stdio: ["ignore", "pipe", "pipe"] },
);
server.stderr.pipe(process.stderr);
(async () => {
  let browser;
  try {
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error("Vite startup timed out")),
        20000,
      );
      server.stdout.on("data", (data) => {
        if (String(data).includes("http://")) {
          clearTimeout(timeout);
          resolve();
        }
      });
      server.once("exit", (code) => {
        clearTimeout(timeout);
        reject(new Error(`Vite exited ${code}`));
      });
    });
    browser = await chromium.launch({
      headless: true,
      executablePath: process.env.LUGUS_CHROMIUM,
    });
    const page = await browser.newPage({
      viewport: { width: 1440, height: 960 },
    });
    page.setDefaultTimeout(5000);
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(() => {
      let selected = "prices";
      const views = [
        {
          id: "prices",
          dataset_id: "prices",
          kind: "price_chart",
          descriptor_revision: 1,
        },
        {
          id: "facts",
          dataset_id: "facts",
          kind: "data_table",
          descriptor_revision: 1,
        },
        {
          id: "document",
          dataset_id: "document",
          kind: "document",
          descriptor_revision: 1,
        },
      ];
      const company = { namespace: "sec", value: "320193" };
      window.__TAURI_INTERNALS__ = {
        invoke: async (_cmd, args) => {
          const r = JSON.parse(args.payload);
          switch (r.op) {
            case "info":
              return { offline: true, runtime_available: false };
            case "company":
            case "list":
              return {
                items: [
                  {
                    id: "chat",
                    workspace_id: "workspace",
                    title: "Understanding Apple",
                    created_at: "2026-09-15",
                  },
                ],
                next_offset: null,
              };
            case "messages":
              return {
                items: [
                  {
                    id: "answer",
                    conversation_id: "chat",
                    run_id: "run",
                    role: "assistant",
                    text: "## Apple at a glance\nExplore the saved price history and reported revenue to understand this business.",
                    created_at: "2026-09-15",
                  },
                ],
                next_offset: null,
              };
            case "runs":
              return { items: [], next_offset: null };
            case "workspace":
              return {
                revision: 1,
                view_ids: views.map((v) => v.id),
                selected_view_id: selected,
              };
            case "view":
              return views.find((v) => v.id === r.view);
            case "select":
              selected = r.view;
              return {};
            case "presented":
              return {};
            case "binding":
              return {
                record: {
                  company: {
                    candidate: { identifier: company, name: "Apple Inc." },
                  },
                  listing: {
                    ticker: { namespace: "ticker", value: "AAPL" },
                    exchange: { namespace: "exchange", value: "NASDAQ" },
                  },
                },
                status: { status: "resolved" },
              };
            case "read":
              return {
                header: {
                  id: r.view,
                  kind: r.view === "prices" ? "prices" : "facts",
                  binding_id: "apple",
                  row_count: 1,
                  query: { company, concept: "Revenues" },
                  created_at: "2026-09-15T10:00:00Z",
                  provider: {
                    instance_id: "fixture",
                    plugin_id: "fixture",
                    plugin_version: "1",
                  },
                  limitations: [],
                  conflicts: [],
                  error: null,
                  coverage: null,
                },
                rows:
                  r.view === "prices"
                    ? [
                        {
                          kind: "price",
                          value: "200",
                          evidence: {
                            value: {
                              date: "2026-09-14",
                              currency: "USD",
                              close: "200",
                              retrieved_at: "2026-09-15",
                              source_url: "https://example.test/prices",
                              instrument: company,
                            },
                          },
                        },
                      ]
                    : [
                        {
                          kind: "fact",
                          group: {
                            period: {
                              kind: "duration",
                              start: "2025-01-01",
                              end: "2025-12-31",
                            },
                            filed: "2026-02-01",
                            value: "100000000",
                            conflict: null,
                            candidates: [
                              {
                                value: {
                                  company,
                                  concept: "Revenues",
                                  label: "Revenue",
                                  unit: "USD",
                                  source_url: "https://example.test/filing",
                                  retrieved_at: "2026-09-15",
                                },
                              },
                            ],
                          },
                        },
                      ],
                next_offset: null,
              };
            default:
              throw new Error("Unexpected operation " + r.op);
          }
        },
      };
    });
    await page.goto("http://127.0.0.1:4198");
    await page.getByRole("button", {name:"Research", exact:true}).click();
    await page
      .getByRole("button", { name: "Understanding Apple", exact: true })
      .click();
    await page
      .locator("#company-name")
      .filter({ hasText: "Apple Inc." })
      .waitFor();
    await page.locator("#message-input").fill("How did revenue change?");
    await page.locator("#toggle-research").click();
    // A context chip must reveal the exact selected evidence and move keyboard focus there.
    await page
      .locator("#selected-context")
      .getByRole("button", { name: /Apple Inc.*Price history/ })
      .click();
    await page.waitForFunction(
      () => document.activeElement?.id === "company-name",
    );
    assert.ok(await page.locator("#research").isVisible());
    await page
      .getByRole("button", { name: "Back to question", exact: true })
      .click();
    await page.waitForFunction(
      () => document.activeElement?.id === "message-input",
    );
    // Picking evidence changes the real composer context, without clearing a draft.
    await page
      .getByRole("navigation", { name: "Conversation evidence" })
      .getByRole("button", { name: /Apple Inc.*Revenues/ })
      .click();
    await page.waitForFunction(() =>
      document
        .querySelector("#selected-context")
        ?.textContent.includes("Revenues"),
    );
    assert.equal(
      await page.locator("#message-input").inputValue(),
      "How did revenue change?",
    );
    assert.ok(
      (await page.locator(".research-snapshot").innerText()).includes(
        "2026-09-15",
      ),
    );
    assert.ok(
      await page
        .getByRole("tabpanel", { name: "Financials", exact: true })
        .isVisible(),
    );
    await page.screenshot({
      path: "/tmp/lugus-research-polish-desktop.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 600, height: 900 });
    await page
      .getByRole("button", { name: "Back to question", exact: true })
      .click();
    assert.ok(await page.locator("#message-input").isVisible());
    await page
      .locator("#selected-context")
      .getByRole("button", { name: /Apple Inc.*Revenues/ })
      .click();
    await page.waitForFunction(
      () => document.activeElement?.id === "company-name",
    );
    assert.ok(await page.locator("#research").isVisible());
    await page.screenshot({
      path: "/tmp/lugus-research-polish-narrow.png",
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Back to question", exact: true })
      .click();
    assert.equal(
      await page.locator("#message-input").inputValue(),
      "How did revenue change?",
    );
    await page
      .getByRole("navigation", { name: "Conversation evidence" })
      .getByRole("button", { name: "Saved filing document", exact: true })
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector("#company-name")?.textContent ===
        "Saved filing document",
    );
    assert.equal(await page.locator(".research-snapshot").count(), 0);
    await page
      .getByRole("button", { name: "Back to question", exact: true })
      .click();
    await page
      .getByRole("button", {
        name: "Remove selected view from message context",
      })
      .click();
    assert.equal(await page.locator("#selected-context").count(), 0);
    assert.ok(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    );
    assert.deepEqual(errors, []);
    console.log(
      "PASS: evidence selection, context inspection, focus return, snapshot metadata, document fallback, draft preservation, narrow layout.",
    );
  } finally {
    await browser?.close();
    server.kill();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
