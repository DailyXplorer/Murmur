import { expect, test, type Page } from "@playwright/test";
import { DEFAULT_WINDOW } from "./windowSizes";

const fixture = "/tests/fixtures/settings-pages.html";
const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

const SCREENS: { file: string; label: string; query: string }[] = [
  { file: "general", label: "General", query: "" },
  { file: "transcription", label: "Transcription", query: "engine=local" },
  {
    file: "transcription-no-account",
    label: "Transcription",
    query: "engine=local&account=none&models=missing",
  },
  { file: "advanced", label: "Advanced", query: "mcp=1" },
  { file: "meetings", label: "Meetings", query: "meetings=1" },
  {
    file: "meetings-no-system-audio",
    label: "Meetings",
    query: "meetings=1&systemAudio=os_too_old&analysis=unsupported",
  },
  { file: "history", label: "History", query: "" },
  { file: "about", label: "About", query: "" },
  { file: "windows-general", label: "General", query: "platform=windows" },
  {
    file: "windows-transcription",
    label: "Transcription",
    query: "platform=windows&engine=local",
  },
  {
    file: "windows-transcription-no-account",
    label: "Transcription",
    query: "platform=windows&engine=local&account=none&models=missing",
  },
  { file: "windows-advanced", label: "Advanced", query: "platform=windows" },
  {
    file: "windows-advanced-mcp",
    label: "Advanced",
    query: "platform=windows&mcp=1&mcpDictations=1",
  },
  { file: "windows-meetings", label: "Meetings", query: "platform=windows" },
  {
    file: "windows-meetings-no-system-audio",
    label: "Meetings",
    query: "platform=windows&systemAudio=unsupported&analysis=unsupported",
  },
  { file: "windows-history", label: "History", query: "platform=windows" },
  { file: "windows-about", label: "About", query: "platform=windows" },
];

const open = async (page: Page, label: string, query: string) => {
  await page.setViewportSize(DEFAULT_WINDOW);
  await page.goto(`${fixture}?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: label, exact: true })
    .click();
  // The sidebar's selection slides over to the section.
  await page.waitForTimeout(500);
};

const growViewportToContent = async (page: Page) => {
  const overflow = await page.evaluate(() =>
    Math.max(
      0,
      ...Array.from(document.querySelectorAll<HTMLElement>("*")).map(
        (element) => element.scrollHeight - element.clientHeight,
      ),
    ),
  );
  const { width, height } = page.viewportSize() ?? DEFAULT_WINDOW;
  await page.setViewportSize({ width, height: height + overflow });
};

const openAdvanced = (page: Page, query = "mcp=1") =>
  open(page, "Advanced", query);

const row = (page: Page, title: string) =>
  page
    .locator("div")
    .filter({ has: page.getByRole("heading", { name: title, exact: true }) })
    .filter({ has: page.locator("button, [aria-haspopup]") })
    .last();

/** Every paragraph a row's text column holds besides its title. */
const paragraphsUnderTitles = (page: Page) =>
  page.evaluate(() =>
    Array.from(document.querySelectorAll("main h3")).flatMap((title) => {
      const column = title.parentElement?.parentElement;
      if (!column) return [];
      return Array.from(column.querySelectorAll("p"))
        .filter((paragraph) => !paragraph.hasAttribute("data-live-status"))
        .map((paragraph) => `${title.textContent}: ${paragraph.textContent}`);
    }),
  );

for (const screen of SCREENS) {
  test(`no row of ${screen.file} explains itself under its title`, async ({
    page,
  }) => {
    await open(page, screen.label, screen.query);

    expect(await paragraphsUnderTitles(page)).toEqual([]);
    expect(
      await page.evaluate(() => window.settingsPages.unhandledCommands),
    ).toEqual([]);

    if (outputDir) {
      await growViewportToContent(page);
      await page.screenshot({ path: `${outputDir}/${screen.file}.png` });
    }
  });
}

test("Delete meeting audio keeps its explanation and the size in its bubble", async ({
  page,
}) => {
  await openAdvanced(page);
  const audio = row(page, "Delete meeting audio");

  await expect(audio.getByText("Transcripts and summaries")).toHaveCount(0);
  await expect(audio.getByText(/Meeting audio uses/)).toHaveCount(0);
  await audio.getByRole("button", { name: /^Deletes the audio/ }).click();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Deletes the audio of processed meetings after the chosen time, to save space. Transcripts and summaries are kept. Meeting audio uses 1.2 GB.",
  );

  if (outputDir) {
    await audio.scrollIntoViewIfNeeded();
    // The bubble fades in.
    await page.waitForTimeout(500);
    await page.screenshot({
      path: `${outputDir}/delete-meeting-audio-tooltip.png`,
    });
  }
});

test("the MCP rows keep a short explanation in their bubbles", async ({
  page,
}) => {
  await openAdvanced(page);
  const mcp = row(page, "Let AI agents read your meetings (MCP)");

  await expect(mcp.getByText(/may send what they read/)).toHaveCount(0);
  await mcp.getByRole("button", { name: /^Lets AI apps you connect/ }).click();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Lets AI apps you connect, such as Claude or Codex, list, search and read your meetings. Read-only. They may send what they read to their own servers.",
  );

  if (outputDir) {
    await mcp.scrollIntoViewIfNeeded();
    // The bubble fades in.
    await page.waitForTimeout(500);
    await page.screenshot({ path: `${outputDir}/mcp-tooltip.png` });
  }
});
