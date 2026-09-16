const { chromium } = require(
  process.env.LUGUS_PLAYWRIGHT_MODULE || "playwright",
);
const fs = require("node:fs"),
  http = require("node:http"),
  cp = require("node:child_process"),
  readline = require("node:readline"),
  path = require("node:path"),
  assert = require("node:assert/strict");
const dir = fs.mkdtempSync("/tmp/lugus-company-browser-");
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
  process.env.LUGUS_COMPANY_QA_BINARY ||
    path.resolve(__dirname, "../src-tauri/target/debug/examples/company_qa"),
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
      .getByLabel("Investment thesis", { exact: true })
      .fill("Pricing power and disciplined reinvestment.");
    await page
      .getByLabel("Open research questions", { exact: true })
      .fill("Are margins structural?");
    await page
      .getByRole("button", { name: "Save research brief", exact: true })
      .click();
    await page.getByText("Brief revision 1", { exact: true }).waitFor();
    await page
      .getByRole("button", { name: "Research with agent", exact: true })
      .click();
    await page
      .getByText("Using saved company brief", { exact: true })
      .waitFor();
    await page.getByLabel("Message Lugus", { exact: true }).fill("Apple draft");
    await page
      .getByRole("button", { name: "Company brief", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Add company", exact: true })
      .click();
    await page.getByLabel("Company name", { exact: true }).fill("Microsoft");
    await page
      .getByLabel("Company or ticker hint", { exact: true })
      .fill("MSFT");
    await page
      .getByRole("button", { name: "Create company", exact: true })
      .click();
    await page
      .getByText("MSFT · Saved research workspace", { exact: true })
      .waitFor();
    await page.getByLabel("Investment thesis").waitFor();
    assert.equal(await page.getByLabel("Investment thesis").inputValue(), "");
    await page
      .getByRole("navigation", { name: "Companies", exact: true })
      .getByRole("button", { name: "Apple", exact: true })
      .click();
    await page
      .getByText("AAPL · Saved research workspace", { exact: true })
      .waitFor();
    await page.getByLabel("Investment thesis").waitFor();
    assert.equal(
      await page.getByLabel("Investment thesis").inputValue(),
      "Pricing power and disciplined reinvestment.",
    );
    await page
      .getByLabel("Open research questions")
      .fill("Unsaved Apple question");
    await page
      .getByRole("navigation", { name: "Companies", exact: true })
      .getByRole("button", { name: "Microsoft", exact: true })
      .click();
    await page.getByLabel("Investment thesis").waitFor();
    await page
      .getByRole("navigation", { name: "Companies", exact: true })
      .getByRole("button", { name: "Apple", exact: true })
      .click();
    await page.getByLabel("Investment thesis").waitFor();
    assert.equal(
      await page.getByLabel("Open research questions").inputValue(),
      "Unsaved Apple question",
    );
    await page
      .getByRole("button", { name: "Save research brief", exact: true })
      .click();
    await page.getByText("Brief revision 2", { exact: true }).waitFor();
    await page
      .getByRole("button", { name: "Review history", exact: true })
      .click();
    await page.getByText("Revision 1 ·", { exact: false }).waitFor();
    await page.screenshot({
      path: path.join(dir, "history.png"),
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Review thesis", exact: true })
      .click();
    assert.match(
      await page.getByLabel("Message Lugus").inputValue(),
      /Apple draft[\s\S]*Review this company/,
    );
    await page
      .getByText("Thesis review · sending may retrieve fresh evidence", {
        exact: true,
      })
      .waitFor();
    await page.locator("#send").click();
    await page
      .getByRole("button", { name: "Save finding", exact: true })
      .waitFor();
    await page
      .getByRole("button", { name: "Save finding", exact: true })
      .click();
    await page
      .getByLabel("Finding text")
      .fill("Pricing power deserves further investigation.");
    await page
      .getByRole("button", { name: "Accept finding", exact: true })
      .click();
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await page
      .getByRole("button", { name: "Company brief", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Accepted findings (1)", exact: true })
      .click();
    await page
      .getByText("Pricing power deserves further investigation.", {
        exact: true,
      })
      .waitFor();
    await page
      .getByRole("button", { name: "Open original answer", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Save finding", exact: true })
      .waitFor();
    await page
      .getByRole("button", { name: "Company brief", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Review history", exact: true })
      .click();
    await page.getByText(/· completed/).waitFor();
    await page.screenshot({
      path: path.join(dir, "completed-review.png"),
      fullPage: true,
    });
    await page.reload();
    await page
      .getByRole("navigation", { name: "Companies", exact: true })
      .getByRole("button", { name: "Apple", exact: true })
      .click();
    await page.getByLabel("Investment thesis").waitFor();
    assert.equal(
      await page.getByLabel("Investment thesis").inputValue(),
      "Pricing power and disciplined reinvestment.",
    );
    await page.screenshot({
      path: path.join(dir, "workbench.png"),
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await page
      .getByRole("button", { name: "Open navigation", exact: true })
      .click();
    await page
      .getByRole("navigation", { name: "Companies", exact: true })
      .getByRole("button", { name: "Microsoft", exact: true })
      .click();
    await page.getByLabel("Investment thesis").waitFor();
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
      true,
    );
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await page.screenshot({
      path: path.join(dir, "mobile.png"),
      fullPage: true,
      animations: "disabled",
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
