const { chromium } = require(
  process.env.LUGUS_PLAYWRIGHT_MODULE || "playwright",
);
const fs = require("node:fs"),
  http = require("node:http"),
  cp = require("node:child_process"),
  readline = require("node:readline"),
  path = require("node:path"),
  assert = require("node:assert/strict");
const dir = fs.mkdtempSync("/tmp/lugus-comparison-browser-");
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
  process.env.LUGUS_COMPARISON_QA_BINARY ||
    path.resolve(__dirname, "../src-tauri/target/debug/examples/comparison_qa"),
  [dir],
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
  if (!file.startsWith(root + "/")) {
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
    page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
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
    await page.getByLabel("Company name", { exact: true }).fill("Apple");
    await page
      .getByLabel("Company or ticker hint", { exact: true })
      .fill("AAPL");
    await page
      .getByRole("button", { name: "Create company", exact: true })
      .click();
    await page
      .getByLabel("Investment thesis")
      .fill("Unsaved thesis stays intact");
    await page
      .getByRole("button", { name: "Compare companies", exact: true })
      .click();
    await page.getByLabel("Second company", { exact: true }).fill("MSFT");
    await page.getByLabel("Reporting endpoint").fill("2024-12-31");
    await page
      .getByRole("button", { name: "Create comparison", exact: true })
      .click();
    await page
      .getByText("Requested calculations available", { exact: false })
      .waitFor();
    const versions = page.getByLabel("Saved comparison", { exact: true });
    const original = await versions.inputValue();
    const table = page.locator(".comparison-company").first();
    assert.match(await table.innerText(), /120/);
    const inspect = page
      .getByRole("button", { name: /Inspect .* 2024-12-31/ })
      .first();
    await inspect.click();
    const details = page.getByRole("dialog", {
      name: "Comparison sources",
      exact: true,
    });
    await details
      .getByText("Exact percentage ratio:", { exact: false })
      .first()
      .waitFor();
    await details
      .getByText("Evidence identifiers", { exact: true })
      .first()
      .click();
    const originalEvidence = await details.innerText();
    assert.match(originalEvidence, /Observation:/);
    await page.screenshot({
      path: path.join(dir, "sources.png"),
      fullPage: true,
    });
    await details.getByRole("button", { name: "Close", exact: true }).click();
    assert.equal(
      await inspect.evaluate((el) => el === document.activeElement),
      true,
    );
    await rpc(JSON.stringify({ qa_mode: "changed" }));
    await page
      .getByRole("button", { name: "Refresh comparison", exact: true })
      .click();
    await page.waitForFunction(
      (old) =>
        document.querySelector(".comparison-actions select")?.value !== old,
      original,
    );
    await page
      .getByText("Requested calculations available", { exact: false })
      .waitFor();
    assert.match(await table.innerText(), /150/);
    await versions.selectOption(original);
    await page.waitForFunction(() =>
      document
        .querySelector(".comparison-company")
        ?.textContent.includes("120"),
    );
    await page.screenshot({
      path: path.join(dir, "comparison.png"),
      fullPage: true,
    });
    await rpc(JSON.stringify({ qa_mode: "partial" }));
    await page
      .getByRole("button", { name: "Refresh comparison", exact: true })
      .click();
    await page
      .getByText(
        "No eligible annual observations were captured for this company.",
        { exact: true },
      )
      .waitFor();
    await page.setViewportSize({ width: 600, height: 900 });
    const narrow = await page.locator(".comparison-dialog").evaluate((el) => {
      const r = el.getBoundingClientRect();
      return {
        left: r.left,
        right: r.right,
        width: r.width,
        scroll: el.scrollWidth,
        client: el.clientWidth,
        css: getComputedStyle(el).width,
        min: getComputedStyle(el).minWidth,
        viewport: innerWidth,
      };
    });
    assert(
      narrow.left >= 0 &&
        narrow.right <= 600 &&
        narrow.scroll <= narrow.client + 1,
      JSON.stringify(narrow),
    );

    await page.screenshot({
      path: path.join(dir, "partial-narrow.png"),
      fullPage: true,
    });
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    );
    await inspect.click();
    await details.waitFor();
    await details.getByRole("button", { name: "Close", exact: true }).click();
    assert.equal(
      await inspect.evaluate((el) => el === document.activeElement),
      true,
    );
    await rpc(JSON.stringify({ qa_mode: "offline" }));
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page
      .getByRole("dialog", { name: "Compare companies", exact: true })
      .getByRole("button", { name: "Close", exact: true })
      .click();
    assert.equal(
      await page.getByLabel("Investment thesis").inputValue(),
      "Unsaved thesis stays intact",
    );
    await page.reload();
    await page
      .getByRole("navigation", { name: "Companies", exact: true })
      .getByRole("button", { name: "Apple", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Compare companies", exact: true })
      .click();
    await versions.selectOption(original);
    await page.waitForFunction(() =>
      document
        .querySelector(".comparison-company")
        ?.textContent.includes("120"),
    );
    await inspect.click();
    await details
      .getByText("Evidence identifiers", { exact: true })
      .first()
      .click();
    assert.equal(await details.innerText(), originalEvidence);
    await details.getByRole("button", { name: "Close", exact: true }).click();
    assert.equal(
      await page
        .getByRole("button", { name: "Refresh comparison", exact: true })
        .isDisabled(),
      true,
    );
    await page.screenshot({
      path: path.join(dir, "offline.png"),
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
