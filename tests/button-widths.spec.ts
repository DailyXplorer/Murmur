import { expect, test, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const fixture = "/tests/fixtures/settings-pages.html";

const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
type Translation = Record<string, unknown>;
const translations = Object.fromEntries(
  readdirSync(localesDir).map((locale) => [
    locale,
    JSON.parse(
      readFileSync(`${localesDir}${locale}/translation.json`, "utf8"),
    ) as Translation,
  ]),
);
const lookup = (locale: string, key: string) =>
  key
    .split(".")
    .reduce<unknown>(
      (node, part) => (node as Translation | undefined)?.[part],
      translations[locale],
    ) as string;

/** Every label a text button shows, with whether an icon sits before it. */
const TEXT_BUTTON_LABELS: [key: string, icon?: "icon"][] = [
  ["common.open"],
  ["common.copy"],
  ["common.clear"],
  ["common.delete"],
  ["onboarding.permissions.accessibility.action"],
  ["onboarding.permissions.grant"],
  ["onboarding.provider.setupCodex"],
  ["onboarding.provider.retry"],
  ["onboarding.provider.continue"],
  ["settings.transcription.installAntigravity"],
  ["settings.transcription.openAntigravity"],
  ["settings.history.openFolder", "icon"],
  ["settings.history.retry", "icon"],
  ["settings.debug.whatsNewPreview.button"],
  ["settings.debug.liveLogs.resume"],
  ["settings.debug.liveLogs.pause"],
  ["settings.debug.liveLogs.copied"],
  ["settings.about.supportDevelopment.button"],
  ["settings.about.sourceCode.button"],
  ["settings.about.modelLicences.button"],
  ["meetings.start"],
  ["meetings.stop"],
  ["meetings.discard"],
  ["meetings.discardDialog.keep"],
  ["meetings.cancel"],
  ["meetings.permission.open"],
  ["meetings.list.loadMore"],
  ["meetings.renameDialog.save"],
  ["meetings.models.download"],
  ["meetings.models.retry"],
  ["meetings.models.delete"],
  ["meetings.processing.processNow"],
  ["meetings.processing.process"],
  ["meetings.processing.retry"],
  ["meetings.speakers.separate"],
  ["meetings.audio.retranscribe"],
  ["meetings.audio.delete"],
];

/** Each label's button width, set on a copy of a real text button. */
const widthsOf = (page: Page, labels: [string, boolean][]) =>
  page.evaluate((labels) => {
    const source = document.querySelector<HTMLElement>(
      'button[data-control="text"]',
    );
    if (!source) throw new Error("no text button on the page");
    return labels.map(([label, icon]) => {
      const button = source.cloneNode(false) as HTMLElement;
      if (icon) {
        const glyph = document.createElement("span");
        glyph.style.cssText = "display:inline-block;width:15px;height:15px";
        button.append(glyph);
      }
      const text = document.createElement("span");
      text.textContent = label;
      button.append(text);
      source.parentElement?.append(button);
      const width = button.getBoundingClientRect().width;
      const shared = parseFloat(getComputedStyle(button).minWidth);
      button.remove();
      return { label, width, shared };
    });
  }, labels);

const openAbout = async (page: Page, locale = "en") => {
  await page.goto(`${fixture}?meetings=1&lang=${locale}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: lookup(locale, "sidebar.about"), exact: true })
    .click();
  await page.locator('button[data-control="text"]').first().waitFor();
};

test("every English text button label fits the one shared width", async ({
  page,
}) => {
  await openAbout(page);
  const widths = await widthsOf(
    page,
    TEXT_BUTTON_LABELS.map(([key, icon]) => [lookup("en", key), !!icon]),
  );
  for (const { label, width, shared } of widths) {
    expect.soft(width, label).toBeCloseTo(shared, 1);
  }
});

/** The visible text buttons: width, the shared width, and whether the label
 * or the button itself is cut off. */
const textButtons = (page: Page) =>
  page.evaluate(() =>
    Array.from(
      document.querySelectorAll<HTMLElement>('button[data-control="text"]'),
    )
      .filter((button) => button.getClientRects().length > 0)
      .map((button) => {
        const box = button.getBoundingClientRect();
        let clippedBy: string | null =
          box.right > window.innerWidth + 0.5 ? "window" : null;
        for (
          let parent = button.parentElement;
          parent && !clippedBy;
          parent = parent.parentElement
        ) {
          const { overflowX } = getComputedStyle(parent);
          if (
            overflowX !== "visible" &&
            box.right > parent.getBoundingClientRect().right + 0.5
          ) {
            clippedBy = parent.className;
          }
        }
        return {
          label: button.textContent ?? "",
          width: box.width,
          shared: parseFloat(getComputedStyle(button).minWidth),
          labelCut: button.scrollWidth > button.clientWidth + 0.5,
          clippedBy,
        };
      }),
  );

const SCREENS: { name: string; query: string; page: string; meeting?: true }[] =
  [
    { name: "Transcription", query: "", page: "sidebar.transcription" },
    { name: "About", query: "", page: "sidebar.about" },
    { name: "Advanced", query: "", page: "sidebar.advanced" },
    { name: "History", query: "", page: "sidebar.history" },
    { name: "Meetings", query: "&meeting=recording", page: "sidebar.meetings" },
    {
      name: "meeting",
      query: "&audioBytes=350623",
      page: "sidebar.meetings",
      meeting: true,
    },
  ];

const visit = async (
  page: Page,
  locale: string,
  screen: (typeof SCREENS)[0],
) => {
  await page.goto(`${fixture}?meetings=1&lang=${locale}${screen.query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: lookup(locale, screen.page), exact: true })
    .click();
  if (screen.meeting) {
    await page.getByRole("button", { name: /Point hebdo produit/ }).click();
    await page.getByRole("tab").nth(1).click();
  }
  await page.waitForTimeout(200);
};

test.describe("at the default window size", () => {
  test.use({ viewport: { width: 680, height: 570 } });

  test("English text buttons all have the shared width", async ({ page }) => {
    for (const screen of SCREENS) {
      await visit(page, "en", screen);
      const buttons = await textButtons(page);
      expect(buttons.length, screen.name).toBeGreaterThan(0);
      for (const { label, width, shared } of buttons) {
        expect.soft(width, `${screen.name}: ${label}`).toBeCloseTo(shared, 1);
      }
    }
  });

  for (const locale of Object.keys(translations)) {
    test(`no text button is cut off in ${locale}`, async ({ page }) => {
      for (const screen of SCREENS) {
        await visit(page, locale, screen);
        for (const button of await textButtons(page)) {
          const where = `${screen.name}: ${button.label}`;
          expect.soft(button.labelCut, where).toBe(false);
          expect.soft(button.clippedBy, where).toBeNull();
          expect
            .soft(button.width, where)
            .toBeGreaterThanOrEqual(button.shared - 0.5);
        }
      }
    });

    test(`text beside a button keeps its room in ${locale}`, async ({
      page,
    }) => {
      await visit(
        page,
        locale,
        SCREENS.find(({ name }) => name === "Transcription")!,
      );
      const status = page.getByText(
        lookup(locale, "settings.transcription.notInstalled"),
        { exact: true },
      );
      expect(
        await status.evaluate((el) => el.scrollWidth <= el.clientWidth + 0.5),
        "Antigravity status is cut off",
      ).toBe(true);

      const field = page.getByPlaceholder(
        lookup(locale, "settings.advanced.customWords.placeholder"),
      );
      const row = field.locator(
        "xpath=ancestor::div[contains(@class, 'px-4')][1]",
      );
      const [fieldBox, rowBox] = await Promise.all([
        field.boundingBox(),
        row.boundingBox(),
      ]);
      expect(fieldBox?.width ?? 0).toBeGreaterThanOrEqual(
        (rowBox?.width ?? 0) * 0.6,
      );
    });
  }
});

test("buttons side by side have the same width", async ({ page }) => {
  await openAbout(page);
  const about = await Promise.all(
    ["Donate", "View on GitHub", "View Licences"].map((name) =>
      page.getByRole("button", { name, exact: true }).boundingBox(),
    ),
  );
  expect(new Set(about.map((box) => box?.width)).size).toBe(1);

  await visit(
    page,
    "en",
    SCREENS.find(({ name }) => name === "meeting")!,
  );
  const footer = await Promise.all(
    ["Transcribe Again", "Delete Audio"].map((name) =>
      page.getByRole("button", { name, exact: true }).boundingBox(),
    ),
  );
  expect(footer[0]?.width).toBe(footer[1]?.width);
});
