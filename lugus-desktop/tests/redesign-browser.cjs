const { chromium } = require(
  process.env.LUGUS_PLAYWRIGHT_MODULE || "playwright",
);
const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const root = path.resolve(
  process.env.LUGUS_UI_DIST || path.join(__dirname, "../dist"),
);
const artifacts = fs.mkdtempSync("/tmp/lugus-redesign-");
const server = http.createServer((req, res) => {
  const file = path.resolve(
    root,
    "." + (req.url === "/" ? "/index.html" : req.url.split("?")[0]),
  );
  if (!file.startsWith(root + path.sep)) {
    res.writeHead(403).end();
    return;
  }
  try {
    res.setHeader(
      "content-type",
      file.endsWith(".js")
        ? "text/javascript"
        : file.endsWith(".css")
          ? "text/css"
          : "text/html",
    );
    res.end(fs.readFileSync(file));
  } catch {
    res.writeHead(404).end();
  }
});
(async () => {
  let browser, page;
  const errors = [];
  try {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    browser = await chromium.launch({
      headless: true,
      executablePath: process.env.LUGUS_CHROMIUM,
      args: ["--no-sandbox"],
    });
    page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    page.setDefaultTimeout(5000);
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(() => {
      window.__TAURI_INTERNALS__ = {
        invoke: async (_cmd, args) => {
          const request = JSON.parse(args.payload);
          switch (request.op) {
            case "info":
              return { offline: true, runtime_available: false };
            case "company":
            case "list":
              return { items: [], next_offset: null };
            case "agent_settings":
              return {
                selected: "codex",
                offline: true,
                runtime_available: false,
                agents: [
                  {
                    id: "codex",
                    label: "Codex",
                    available: false,
                    detail: "Offline mode",
                  },
                  {
                    id: "claude_code",
                    label: "Claude Code",
                    available: false,
                    detail: "Offline mode",
                  },
                ],
              };
            case "portfolio":
              if (request.command.kind === "list")
                return { items: [], next_offset: null, revision: "0" };
              throw new Error("Unexpected portfolio command");
            default:
              throw new Error("Unexpected test operation " + request.op);
          }
        },
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.getByRole("button", {name:"Research", exact:true}).click();
    await page
      .locator("#message-input")
      .fill("Keep this research question while I navigate.");
    await page.locator("#company-hint").fill("AAPL");
    await page.screenshot({
      path: path.join(artifacts, "research-desktop.png"),
      fullPage: true,
      animations: "disabled",
    });
    await page.setViewportSize({ width: 600, height: 900 });
    await page
      .getByRole("button", { name: "Open navigation", exact: true })
      .click();
    await page.getByRole("button", { name: "Portfolio", exact: true }).click();
    await page
      .getByRole("button", { name: "Create portfolio", exact: true })
      .waitFor();
    assert.ok(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    );
    await page
      .getByRole("button", { name: "Open navigation", exact: true })
      .click();
    await page.getByRole("button", { name: "Research", exact: true }).click();
    // A wide-window research selection remains visible after resize until explicitly hidden.
    const toggle = page.locator("#toggle-research");
    await toggle.waitFor();
    if ((await toggle.getAttribute("aria-expanded")) === "true")
      await toggle.click();
    assert.equal(
      await page.locator("#message-input").inputValue(),
      "Keep this research question while I navigate.",
    );
    assert.equal(await page.locator("#company-hint").inputValue(), "AAPL");
    await page
      .getByRole("button", { name: "Show research", exact: true })
      .click();
    await page.getByRole("tab", { name: "Overview", exact: true }).waitFor();
    await page
      .getByRole("button", { name: "Hide research", exact: true })
      .click();
    assert.ok(await page.locator("#message-input").isVisible());
    await page.screenshot({
      path: path.join(artifacts, "research-narrow.png"),
      fullPage: true,
      animations: "disabled",
    });
    await page
      .getByRole("button", { name: "Open navigation", exact: true })
      .click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page
      .getByRole("dialog", { name: "Application settings", exact: true })
      .waitFor();
    await page.locator(".navigation-sheet").waitFor({ state: "hidden" });
    assert.ok(
      await page
        .getByRole("button", { name: "Save agent", exact: true })
        .isDisabled(),
    );
    await page.screenshot({
      path: path.join(artifacts, "settings-narrow.png"),
      fullPage: true,
      animations: "disabled",
    });
    await page.keyboard.press("Escape");
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    assert.ok(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    );
    assert.deepEqual(errors, []);
    console.log(
      JSON.stringify({
        passed: true,
        artifacts,
        checks: [
          "narrow navigation",
          "draft and hint survive workspace navigation",
          "research toggle retains composer",
          "offline agent controls",
          "no horizontal overflow",
          "no page errors",
        ],
      }),
    );
  } catch (error) {
    if (page) {
      console.error(await page.locator("body").innerText());
      await page.screenshot({
        path: path.join(artifacts, "failure.png"),
        fullPage: true,
        animations: "disabled",
      });
    }
    console.error(artifacts);
    console.error(error);
    process.exitCode = 1;
  } finally {
    await browser?.close();
    server.close();
  }
})();
