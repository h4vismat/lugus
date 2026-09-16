const { chromium } = require(
  process.env.LUGUS_PLAYWRIGHT_MODULE || "playwright",
);
const fs = require("node:fs"),
  http = require("node:http"),
  cp = require("node:child_process"),
  readline = require("node:readline"),
  path = require("node:path"),
  assert = require("node:assert/strict");
const dir = fs.mkdtempSync(
  path.join(require("node:os").tmpdir(), "lugus-data-settings-"),
);
fs.writeFileSync(
  path.join(dir, "app.json"),
  JSON.stringify({
    financial_path: "financial.sqlite",
    application_path: "app.sqlite",
    providers: [],
  }),
);
fs.writeFileSync(
  path.join(dir, "desktop.json"),
  JSON.stringify({ application_config: "app.json" }),
);
const child = cp.spawn(
  process.env.LUGUS_QA_BINARY ||
    path.resolve(__dirname, "../src-tauri/target/debug/examples/portfolio_qa"),
  [path.join(dir, "desktop.json")],
  { stdio: ["pipe", "pipe", "inherit"] },
);
let pending = [];
readline.createInterface({ input: child.stdout }).on("line", (line) => {
  const item = pending.shift();
  if (item) item(JSON.parse(line));
});
function rpc(payload) {
  return new Promise((resolve) => {
    pending.push(resolve);
    child.stdin.write(payload + "\n");
  });
}
const root = path.resolve(__dirname, "../dist");
const server = http.createServer(async (req, res) => {
  if (req.url === "/rpc") {
    let body = "";
    for await (const chunk of req) body += chunk;
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify(await rpc(body)));
    return;
  }
  const file = path.resolve(
    root,
    "." + (req.url === "/" ? "/index.html" : req.url.split("?")[0]),
  );
  if (!file.startsWith(root + path.sep)) {
    res.statusCode = 403;
    res.end();
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
    res.statusCode = 404;
    res.end();
  }
});
(async () => {
  let browser, page;
  try {
    await new Promise((r) => server.listen(0, "127.0.0.1", r));
    browser = await chromium.launch({
      headless: true,
      executablePath: process.env.LUGUS_CHROMIUM,
      args: ["--no-sandbox"],
    });
    page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(() => {
      window.__TAURI_INTERNALS__ = {
        invoke: async (cmd, args) => {
          const r = await fetch("/rpc", {
            method: "POST",
            body: args.payload,
          }).then((r) => r.json());
          if (r.error) throw r.error;
          return r.value;
        },
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page
      .getByRole("button", { name: "Data sources", exact: true })
      .click();
    const internet = page.getByLabel("Enable Internet search", { exact: true });
    await internet.waitFor({ timeout: 5000 });
    assert.equal(await internet.isChecked(), false);
    await internet.check();
    await page.getByRole("button", { name: "Save Internet search", exact: true }).click();
    await page.getByText("Saved. Applies to new messages; current messages keep their settings.", { exact: true }).waitFor();
    assert.equal(JSON.parse(fs.readFileSync(path.join(dir, "desktop.agents.json"), "utf8")).allow_web_search, true);
    assert.equal(JSON.parse(fs.readFileSync(path.join(dir, "app.json"), "utf8")).providers.length, 0);
    await page.getByText("Offline mode — internet search is unavailable.", { exact: true }).waitFor();
    await page
      .getByLabel("Contact name", { exact: true })
      .fill("Test Investor");
    await page
      .getByLabel("Contact email", { exact: true })
      .fill("investor@example.test");
    await page
      .getByLabel("Enable SEC company research", { exact: true })
      .check();
    await page.getByRole("button", { name: "Agent", exact: true }).click();
    await page
      .getByRole("button", { name: "Data sources", exact: true })
      .click();
    assert.equal(
      await page.getByLabel("Contact name", { exact: true }).inputValue(),
      "Test Investor",
    );
    await page
      .getByRole("button", { name: "Save data sources", exact: true })
      .click();
    await page
      .getByText("Saved. Restart Lugus to apply these data-source settings.", {
        exact: true,
      })
      .waitFor();
    const config = JSON.parse(
      fs.readFileSync(path.join(dir, "app.json"), "utf8"),
    );
    assert.equal(config.providers.length, 1);
    assert.equal(config.providers[0].active, true);
    assert.equal(
      config.providers[0].config.user_agent,
      "Lugus Test Investor investor@example.test",
    );
    assert.equal(config.financial_path, "financial.sqlite");
    await page.screenshot({
      path: path.join(dir, "settings.png"),
      fullPage: true,
    });
    await page.getByRole("button", { name: "Done", exact: true }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page
      .getByRole("button", { name: "Data sources", exact: true })
      .click();
    await page.getByLabel("Contact name", { exact: true }).waitFor();
    assert.equal(
      await page.getByLabel("Contact email", { exact: true }).inputValue(),
      "investor@example.test",
    );
    assert.equal(
      await page.getByLabel("Enable SEC company research").isChecked(),
      true,
    );
    assert.equal(await internet.isChecked(), true);
    await internet.uncheck();
    await page.getByRole("button", { name: "Save Internet search", exact: true }).click();
    await page.getByText("Saved. Applies to new messages; current messages keep their settings.", { exact: true }).waitFor();
    assert.equal(JSON.parse(fs.readFileSync(path.join(dir, "desktop.agents.json"), "utf8")).allow_web_search, false);
    await page.setViewportSize({ width: 390, height: 844 });
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
      true,
    );
    await page.screenshot({
      path: path.join(dir, "settings-mobile.png"),
      fullPage: true,
    });
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ passed: true, artifacts: dir }));
  } catch (e) {
    console.error(e);
    if (page) {
      console.error(await page.locator("body").innerText());
      await page.screenshot({
        path: path.join(dir, "failure.png"),
        fullPage: true,
      });
      console.error(dir);
    }
    process.exitCode = 1;
  } finally {
    if (browser) await browser.close();
    server.close();
    child.stdin.end();
  }
})();
