const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const path = require("node:path");
const { chromium } = require(
  process.env.LUGUS_PLAYWRIGHT_MODULE || "playwright",
);
const server = spawn(
  process.execPath,
  [
    path.resolve(__dirname, "../node_modules/vite/bin/vite.js"),
    "--host",
    "127.0.0.1",
    "--port",
    "4197",
    "--strictPort",
  ],
  { cwd: path.resolve(__dirname, ".."), stdio: ["ignore", "pipe", "pipe"] },
);
server.stderr.on("data", (chunk) => process.stderr.write(chunk));
(async () => {
  let browser;
  try {
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error("Vite startup timed out")),
        20000,
      );
      server.stdout.on("data", (chunk) => {
        if (String(chunk).includes("http://")) {
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
      ...(process.env.LUGUS_CHROMIUM || process.env.LUGUS_CHROMIUM_EXECUTABLE
        ? {
            executablePath:
              process.env.LUGUS_CHROMIUM ||
              process.env.LUGUS_CHROMIUM_EXECUTABLE,
          }
        : {}),
    });
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("http://127.0.0.1:4197/tests/research-fixture.html");
    await page.getByRole("tab", { name: "Overview", exact: true }).focus();
    assert.ok(
      await page
        .getByRole("tabpanel", { name: "Overview", exact: true })
        .isVisible(),
    );
    await page.keyboard.press("ArrowRight");
    await page.getByRole("tab", { name: "Financials", exact: true }).waitFor();
    await page.waitForFunction(
      () =>
        document
          .getElementById("tab-financials")
          ?.getAttribute("aria-selected") === "true",
    );
    assert.ok(
      await page
        .getByRole("tabpanel", { name: "Financials", exact: true })
        .isVisible(),
    );
    assert.match(await page.locator(".financial-count").innerText(), /55 rows/);
    assert.match(
      await page.locator(".financial-pagination").innerText(),
      /Page 1 of 2/,
    );
    await page.getByRole("button", { name: "Next", exact: true }).click();
    assert.match(
      await page.locator(".financial-pagination").innerText(),
      /Page 2 of 2/,
    );
    await page.getByRole("button", { name: "Previous", exact: true }).click();
    await page.locator(".financial-details summary").first().click();
    assert.ok(
      (await page.locator(".financial-details").first().innerText()).includes(
        "9,007,199,254,740,993.123456789",
      ),
    );
    assert.ok(
      (await page.locator(".financial-details").first().innerText()).includes(
        "https://example.test/annual",
      ),
    );
    await page
      .getByRole("searchbox", { name: "Find a financial metric" })
      .fill("no such metric");
    assert.match(
      await page
        .getByRole("region", { name: "Financial observations" })
        .innerText(),
      /No metrics/,
    );
    await page
      .getByRole("searchbox", { name: "Find a financial metric" })
      .fill("Revenues");
    await page.getByLabel("Financial filings").selectOption("all");
    assert.match(
      await page.locator(".financial-scope").innerText(),
      /All filings/,
    );
    await page.getByRole("tab", { name: "Balance Sheet", exact: true }).click();
    assert.match(
      await page
        .getByRole("region", { name: "Financial observations" })
        .innerText(),
      /No metrics/,
    );
    await page
      .getByRole("tab", { name: "Income Statement", exact: true })
      .click();
    await page.getByRole("tab", { name: "Filings", exact: true }).click();
    assert.ok(
      await page
        .getByRole("tabpanel", { name: "Filings", exact: true })
        .isVisible(),
    );
    await page.getByRole("tab", { name: "Financials", exact: true }).click();
    await page.getByLabel("Saved research").selectOption("prices");
    await page.getByRole("button", { name: "1M", exact: true }).click();
    await page.getByLabel("Saved research").selectOption("financials");
    assert.equal(
      await page
        .getByRole("tab", { name: "Financials", exact: true })
        .getAttribute("aria-selected"),
      "true",
    );
    await page.getByLabel("Saved research").selectOption("prices");
    assert.equal(
      await page
        .getByRole("button", { name: "1M", exact: true })
        .getAttribute("aria-pressed"),
      "true",
    );
    await page.locator("#settings-button").click();
    await page.waitForFunction(() => window.fixture.queue.length === 1);
    assert.match(
      await page.getByRole("dialog").evaluate((element) =>
        (element.getAttribute("aria-describedby") ?? "")
          .split(" ")
          .map((id) => document.getElementById(id)?.textContent)
          .join(" "),
      ),
      /Choose the agent for your next message/,
    );
    await page.locator("#settings-done").click();
    await page.locator("#settings-button").click();
    await page.waitForFunction(() => window.fixture.queue.length === 2);
    await page.evaluate(() => {
      window.fixture.resolve(1);
      window.fixture.resolve(0, {
        ...window.fixture.settings,
        selected: "claude_code",
      });
    });
    await page.waitForFunction(
      () => document.getElementById("settings-agent")?.value === "codex",
    );
    await page.getByLabel("Agent", { exact: true }).selectOption("claude_code");
    await page.locator("#settings-save").click();
    await page.waitForFunction(() => window.fixture.queue.length === 3);
    await page.keyboard.press("Escape");
    assert.ok(await page.getByRole("dialog").isVisible());
    assert.ok(await page.locator("#settings-done").isDisabled());
    await page.evaluate(() => window.fixture.reject(2));
    await page.getByRole("alert").waitFor();
    await page.locator("#settings-save").click();
    await page.waitForFunction(() => window.fixture.queue.length === 4);
    await page.evaluate(() =>
      window.fixture.resolve(3, {
        ...window.fixture.settings,
        selected: "claude_code",
      }),
    );
    await page.waitForFunction(
      () =>
        document.getElementById("saved-agent")?.textContent === "claude_code",
    );
    await page.locator("#settings-done").click();
    await page.waitForFunction(
      () => document.activeElement?.id === "settings-button",
    );
    await page.locator("#settings-button").click();
    await page.waitForFunction(() => window.fixture.queue.length === 5);
    await page.evaluate(() =>
      window.fixture.resolve(4, { ...window.fixture.settings, offline: true }),
    );
    await page.waitForFunction(
      () => document.getElementById("settings-agent")?.disabled === true,
    );
    assert.ok(await page.locator("#settings-save").isDisabled());
    await page.evaluate(() =>
      document.getElementById("settings-button").remove(),
    );
    await page.locator("#settings-done").click();
    await page.waitForFunction(
      () =>
        document.activeElement?.getAttribute("aria-label") ===
        "Open navigation",
    );
    assert.deepEqual(errors, []);
    console.log(
      "PASS: research keyboard tabs, exact source values, filtering, pagination, view preferences; settings stale loads, saving guard, retry, focus restoration, offline state.",
    );
  } finally {
    await browser?.close();
    server.kill();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
