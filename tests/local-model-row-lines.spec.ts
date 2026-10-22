import { readdirSync, readFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { DEFAULT_WINDOW, MIN_WINDOW } from "./windowSizes";

const localesDir = new URL("../src/i18n/locales/", import.meta.url);
const LOCALES = readdirSync(localesDir).sort();
const transcriptionLabel = (locale: string): string =>
  JSON.parse(
    readFileSync(new URL(`${locale}/translation.json`, localesDir), "utf8"),
  ).sidebar.transcription;

/** Every state the row can show, as the settings fixture sets it up. */
const STATES = [
  "models=missing",
  "models=cancelled",
  "models=downloading",
  "models=preparing",
  "models=ready",
  "models=failed",
  "models=serverDown",
  "models=damaged",
  "models=noSpace",
  "models=disk",
  "models=prepareFailed",
  "models=missing&account=none",
  "analysis=unsupported",
  "analysis=unsupported&models=downloading",
  "analysis=os_too_old",
];

/** The lines of text the row's text column takes on screen. */
const rowText = (page: Page) =>
  page.getByTestId("local-model-row").evaluate((row) => {
    const column = row.querySelector(":scope > div > div");
    if (!column) throw new Error("The model row has no text column");
    const middles: number[] = [];
    const walker = document.createTreeWalker(column, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (!node.textContent?.trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      for (const rect of Array.from(range.getClientRects())) {
        if (rect.width === 0 || rect.height === 0) continue;
        const middle = rect.top + rect.height / 2;
        if (!middles.some((line) => Math.abs(line - middle) < rect.height / 2))
          middles.push(middle);
      }
    }
    return {
      lines: middles.length,
      text: (column as HTMLElement).innerText,
      buttons: column.querySelectorAll("button").length,
    };
  });

for (const locale of LOCALES) {
  for (const size of [DEFAULT_WINDOW, MIN_WINDOW]) {
    test(`the local model row takes at most two lines (${locale}, ${size.width}x${size.height})`, async ({
      page,
    }) => {
      const label = transcriptionLabel(locale);
      await page.setViewportSize(size);
      for (const state of STATES) {
        await page.goto(
          `/tests/fixtures/settings-pages.html?lang=${locale}&engine=local&${state}`,
        );
        await page
          .getByRole("navigation")
          .getByRole("button", { name: label, exact: true })
          .click();
        await expect(page.getByTestId("local-model-row")).toBeVisible();
        const { lines, text, buttons } = await rowText(page);
        const at = `${state} at ${size.width}x${size.height}: ${JSON.stringify(text)}`;
        expect(lines, at).toBeGreaterThanOrEqual(1);
        expect(lines, at).toBeLessThanOrEqual(2);
        expect(text, at).not.toMatch(/Parakeet/);
        expect(buttons, `no info button in the text, ${at}`).toBe(0);
      }
    });
  }
}
