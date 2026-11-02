import { expect, test, type Page } from "@playwright/test";
import { DEFAULT_WINDOW, MIN_WINDOW } from "./windowSizes";

const fixture = "/tests/fixtures/settings-pages.html";
const outputDir = process.env.MURMUR_SCREENSHOT_DIR;
const COMMAND = "/Applications/Murmur.app/Contents/MacOS/murmur";

const openAdvanced = async (page: Page, query = "") => {
  await page.goto(`${fixture}${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Advanced", exact: true })
    .click();
};

const group = (page: Page) =>
  page.locator("section").filter({
    has: page.getByRole("heading", { name: "AI assistants", exact: true }),
  });

const access = (page: Page) =>
  group(page).getByRole("checkbox", {
    name: "Let AI agents read your meetings (MCP)",
  });

const dictations = (page: Page) =>
  group(page).getByRole("checkbox", { name: "Include dictation history" });

const calls = (page: Page) => page.evaluate(() => window.settingsPages.calls);

test("AI access is off by default and shows nothing to connect", async ({
  page,
}) => {
  await openAdvanced(page);

  await expect(access(page)).not.toBeChecked();
  await expect(dictations(page)).toHaveCount(0);
  await expect(group(page).getByRole("tablist")).toHaveCount(0);
  await expect(
    group(page).getByRole("button", { name: "Copy setup for your app" }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("turning AI access on saves it and offers the setup", async ({ page }) => {
  await openAdvanced(page);

  await access(page).check({ force: true });

  await expect(access(page)).toBeChecked();
  expect(await calls(page)).toContainEqual({
    command: "change_mcp_enabled_setting",
    args: { enabled: true },
  });
  // Dictations have a switch of their own, off.
  await expect(dictations(page)).not.toBeChecked();
  await expect(group(page).getByTestId("mcp-setup")).toContainText(COMMAND);
});

for (const size of [MIN_WINDOW, DEFAULT_WINDOW]) {
  test(`setup copy action stays right-aligned at ${size.width}px`, async ({
    page,
  }) => {
    await page.setViewportSize(size);
    await openAdvanced(page, "?mcp=1");

    await expect(
      group(page).getByRole("heading", { name: "Connect an app", exact: true }),
    ).toBeVisible();
    const setup = await group(page).getByTestId("mcp-setup").boundingBox();
    const button = await group(page)
      .getByRole("button", { name: "Copy setup for your app", exact: true })
      .boundingBox();
    expect(setup).not.toBeNull();
    expect(button).not.toBeNull();
    expect(
      Math.abs(button!.x + button!.width - setup!.x - setup!.width),
    ).toBeLessThanOrEqual(1);
  });
}

test("dictation history is a second switch", async ({ page }) => {
  await openAdvanced(page, "?mcp=1");

  await dictations(page).check({ force: true });

  await expect(dictations(page)).toBeChecked();
  expect(await calls(page)).toContainEqual({
    command: "change_mcp_include_dictations_setting",
    args: { enabled: true },
  });
});

test("the setup shows the exact command for each AI app", async ({ page }) => {
  await openAdvanced(page, "?mcp=1");
  const setup = group(page).getByTestId("mcp-setup");

  await expect(setup).toHaveText(`claude mcp add murmur -- '${COMMAND}' mcp`);
  await group(page).getByRole("tab", { name: "Codex" }).click();
  await expect(setup).toHaveText(
    `[mcp_servers.murmur]\ncommand = "${COMMAND}"\nargs = ["mcp"]`,
  );
  await group(page).getByRole("tab", { name: "Claude Desktop" }).click();
  expect(JSON.parse((await setup.textContent()) ?? "")).toEqual({
    mcpServers: { murmur: { command: COMMAND, args: ["mcp"] } },
  });
});

test("the setup is copied to the clipboard", async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(window, { copiedText: "" });
    navigator.clipboard.writeText = (text) => {
      Object.assign(window, { copiedText: text });
      return Promise.resolve();
    };
  });
  await openAdvanced(page, "?mcp=1");
  await group(page).getByRole("tab", { name: "Codex" }).click();

  await group(page)
    .getByRole("button", { name: "Copy setup for your app" })
    .click();

  await expect(
    group(page).getByRole("button", { name: "Copied" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { copiedText: string }).copiedText,
    ),
  ).toBe(`[mcp_servers.murmur]\ncommand = "${COMMAND}"\nargs = ["mcp"]`);
});

for (const theme of ["light", "dark"] as const) {
  for (const state of ["off", "on"] as const) {
    test(`screenshot of AI access ${state} (${theme})`, async ({ page }) => {
      test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize(DEFAULT_WINDOW);
      await openAdvanced(
        page,
        `?theme=${theme}${state === "on" ? "&mcp=1" : ""}`,
      );
      await expect(group(page)).toBeVisible();
      await group(page).scrollIntoViewIfNeeded();
      // The sidebar's selection slides over to Advanced.
      await page.waitForTimeout(500);
      await group(page).screenshot({
        path: `${outputDir}/mcp-access-${state}-${theme}.png`,
      });
    });
  }
}
