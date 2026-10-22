import { expect, test, type Locator, type Page } from "@playwright/test";
import { readdirSync } from "node:fs";
import { DEFAULT_WINDOW, MIN_WINDOW } from "./windowSizes";

const LOCALES = readdirSync(new URL("../src/i18n/locales/", import.meta.url));

const SECTIONS = [
  "General",
  "Transcription",
  "History",
  "Meetings",
  "Insights",
];

const openSection = async (
  page: Page,
  section: string,
  query = "",
  size = MIN_WINDOW,
) => {
  await page.setViewportSize(size);
  await page.goto(`/tests/fixtures/settings-pages.html${query}`);
  const button = page
    .getByRole("navigation")
    .getByRole("button")
    .nth(SECTIONS.indexOf(section));
  await button.click();
  await expect(button).toHaveAttribute("aria-current", "page");
};

const box = async (locator: Locator) => {
  const value = await locator.boundingBox();
  if (value === null) throw new Error("element is not visible");
  return value;
};

/** The setting row whose title is `title`. */
const settingRow = (page: Page, title: string) =>
  page
    .getByRole("heading", { name: title, exact: true })
    .locator("xpath=ancestor::div[contains(@class,'px-4')][1]");

const isTruncated = (locator: Locator) =>
  locator.evaluate((element) => element.scrollWidth > element.clientWidth);

test("Codex row stacks its status under the title", async ({ page }) => {
  await openSection(page, "Transcription");
  const row = settingRow(page, "Codex");
  const title = await box(row.getByRole("heading"));
  const status = row.getByText("Configuration detected", { exact: true });

  expect((await box(status)).y).toBeGreaterThanOrEqual(title.y + title.height);
  expect(await isTruncated(status)).toBe(false);
});

test("Custom words puts a full-width field under its title", async ({
  page,
}) => {
  await openSection(page, "Transcription");
  const row = settingRow(page, "Custom words");
  const title = await box(row.getByRole("heading"));
  const field = await box(row.getByPlaceholder("Add a word"));
  const add = row.getByRole("button", { name: "Add", exact: true });
  const addBox = await box(add);
  const rowBox = await box(row);

  expect(field.y).toBeGreaterThanOrEqual(title.y + title.height);
  expect(Math.abs(field.y - addBox.y)).toBeLessThan(2);
  expect(field.x - rowBox.x).toBeLessThan(20);
  expect(rowBox.x + rowBox.width - (addBox.x + addBox.width)).toBeLessThan(20);
  await expect(add).toHaveAttribute("data-control", "text");

  await row.getByPlaceholder("Add a word").fill("Kubernetes");
  await row.getByPlaceholder("Add a word").press("Enter");
  await expect(row.getByPlaceholder("Add a word")).toHaveValue("");
});

const statTiles = (page: Page) =>
  // The stat cards are the sections that show a value right under the label.
  page.getByRole("main").locator("section:has(> p)");

for (const [name, size] of [
  ["default", DEFAULT_WINDOW],
  ["minimum", MIN_WINDOW],
] as const) {
  for (const language of ["en", "de", "es", "fr", "it", "pt"]) {
    test(`Insights stat labels fit in full (${language}, ${name} window)`, async ({
      page,
    }) => {
      await openSection(page, "Insights", `?lang=${language}`, size);
      const tiles = statTiles(page);
      await expect(tiles).toHaveCount(3);
      const overflows = await tiles.evaluateAll((elements) =>
        elements.flatMap((tile) => {
          const padding = parseFloat(getComputedStyle(tile).paddingRight);
          const right = tile.getBoundingClientRect().right - padding;
          const label = tile.querySelector("h2")!;
          const parts = [
            ...Array.from(label.getClientRects()),
            ...Array.from(tile.querySelectorAll("svg, button")).map((element) =>
              element.getBoundingClientRect(),
            ),
          ];
          return [
            ...(label.scrollWidth > label.clientWidth
              ? [`${label.textContent} is cut off`]
              : []),
            ...parts
              .filter((rect) => rect.right > right + 0.5)
              .map(
                (rect) =>
                  `${label.textContent} reaches ${rect.right - right}px past the padding`,
              ),
          ];
        }),
      );
      expect(overflows).toEqual([]);
    });
  }
}

/** Every way a stat value fails to sit on one line inside its tile. */
const statValueProblems = (tiles: Locator, language: string) =>
  tiles.evaluateAll((elements, language) => {
    const huge = new Intl.NumberFormat(language).format(999);
    const values = elements.map((tile) => tile.querySelector(":scope > p")!);
    const problems = values.flatMap((value, index) => {
      const tile = elements[index];
      const text = value.textContent ?? "";
      const style = getComputedStyle(value);
      const tileStyle = getComputedStyle(tile);
      const tileBox = tile.getBoundingClientRect();
      const left = tileBox.left + parseFloat(tileStyle.paddingLeft);
      const right = tileBox.right - parseFloat(tileStyle.paddingRight);
      const inner = document.createRange();
      inner.selectNodeContents(value);
      const innerBox = inner.getBoundingClientRect();
      // Mixed-direction text gets one rect per bidi run on the same line.
      const lines = new Set(
        Array.from(inner.getClientRects(), (rect) => Math.round(rect.top)),
      ).size;
      const height = value.getBoundingClientRect().height;
      return [
        ...(text.includes(huge) ? [] : [`${text} does not show ${huge}`]),
        ...(value.scrollWidth > value.clientWidth
          ? [`${text} overflows its box`]
          : []),
        ...(lines === 1 && height <= parseFloat(style.lineHeight) + 1
          ? []
          : [`${text} takes ${lines} lines, ${height}px high`]),
        ...(innerBox.left < left - 0.5 || innerBox.right > right + 0.5
          ? [`${text} reaches past the tile padding`]
          : []),
      ];
    });
    const tops = values.map((value) => value.getBoundingClientRect().top);
    const heights = elements.map((tile) => tile.getBoundingClientRect().height);
    if (Math.max(...tops) - Math.min(...tops) > 1) {
      problems.push(`values sit at different heights: ${tops.join(", ")}`);
    }
    if (Math.max(...heights) - Math.min(...heights) > 0.5) {
      problems.push(`tiles differ in height: ${heights.join(", ")}`);
    }
    return problems;
  }, language);

const expectStatValuesOnOneLine = async (
  page: Page,
  language: string,
  theme: "light" | "dark",
) => {
  await page.emulateMedia({ colorScheme: theme });
  await openSection(
    page,
    "Insights",
    `?insights=huge&lang=${language}&theme=${theme}`,
    DEFAULT_WINDOW,
  );
  const tiles = statTiles(page);
  await expect(tiles).toHaveCount(3);
  await page.evaluate(() => document.fonts.ready);
  for (const size of [DEFAULT_WINDOW, MIN_WINDOW]) {
    await page.setViewportSize(size);
    await expect
      .poll(
        () => statValueProblems(tiles, language),
        `${theme}, ${size.width}px`,
      )
      .toEqual([]);
  }
};

for (const language of LOCALES) {
  test(`Insights stat values stay on one line in their tiles (${language})`, async ({
    page,
  }) => {
    for (const theme of ["light", "dark"] as const) {
      await expectStatValuesOnOneLine(page, language, theme);
    }
  });

  // Linux draws text wider than macOS: wider fallback fonts, and glyph
  // advances rounded up to whole pixels, which do not shrink with the font
  // size. A wide font with a fixed spacing between letters reproduces both,
  // so the fit cannot rely on the metrics of one machine. The widest spacing
  // leaves no room for the full values even at the smallest scale.
  for (const [width, spacing] of [
    ["wide", "1px"],
    ["very wide", "8px"],
  ]) {
    test(`Insights stat values stay on one line in their tiles with a ${width} font (${language})`, async ({
      page,
    }) => {
      await page.addInitScript((spacing) => {
        const sheet = new CSSStyleSheet();
        sheet.replaceSync(`main section > p {
          font-family: "DejaVu Sans", Verdana, sans-serif !important;
          letter-spacing: ${spacing};
        }`);
        document.adoptedStyleSheets = [sheet];
      }, spacing);
      await expectStatValuesOnOneLine(page, language, "light");
    });
  }
}

test("Insights has no subtitles or footnote", async ({ page }) => {
  await openSection(page, "Insights");
  const main = page.getByRole("main");
  await expect(main.getByText("Usage by app", { exact: true })).toBeVisible();
  for (const text of [
    "Where your dictations went, by app category.",
    "Days with at least one dictation, in your local time zone.",
    "Insights are calculated on this Mac and stay here. No dictated text is stored for them.",
  ]) {
    await expect(main.getByText(text)).toHaveCount(0);
  }
});

const toolbarSpacing = (buttons: Locator) =>
  buttons.evaluateAll((elements) => {
    const boxes = elements.map((element) => element.getBoundingClientRect());
    return {
      sizes: boxes.map((rect) => [rect.width, rect.height]),
      gaps: boxes.slice(1).map((rect, index) => rect.left - boxes[index].right),
    };
  });

test("History entry actions match the meeting header toolbar", async ({
  page,
}) => {
  await openSection(page, "History");
  const entryActions = page
    .getByRole("button", { name: "Copy transcription to clipboard" })
    .first()
    .locator("xpath=..")
    .getByRole("button");
  const history = await toolbarSpacing(entryActions);

  await openSection(page, "Meetings", "?meetings=1");
  await page.getByRole("button", { name: /Point hebdo produit/ }).click();
  const meetingActions = page
    .getByRole("button", { name: "Search summary" })
    .locator("xpath=..")
    .getByRole("button");
  const meeting = await toolbarSpacing(meetingActions);

  expect(history.sizes).toHaveLength(4);
  for (const size of history.sizes) expect(size).toEqual(meeting.sizes[0]);
  for (const gap of history.gaps) expect(gap).toBe(meeting.gaps[0]);
});

for (const theme of ["light", "dark"] as const) {
  test(`selected accent swatch keeps a light ring inside its row (${theme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: theme });
    await openSection(page, "General", `?theme=${theme}`);
    const row = settingRow(page, "Accent color");
    const selected = row.getByRole("radio", { checked: true });
    const indicator = await selected.evaluate((input) => {
      const control = input.parentElement!;
      const swatch = input.nextElementSibling as HTMLElement;
      const rowBox = control.closest("div.px-4")!.getBoundingClientRect();
      const shadows = [control, swatch].map(
        (element) => getComputedStyle(element).boxShadow,
      );
      const spreads = shadows
        .flatMap((shadow) => [...shadow.matchAll(/(-?[\d.]+)px(?=,|$)/g)])
        .map((match) => Number(match[1]));
      const swatchBox = swatch.getBoundingClientRect();
      const reach = Math.max(0, ...spreads);
      const probe = document.createElement("span");
      probe.style.color = "var(--color-text)";
      document.body.append(probe);
      const textColor = getComputedStyle(probe).color;
      probe.remove();
      return {
        textColor,
        shadows: shadows.join(" "),
        top: swatchBox.top - reach - rowBox.top,
        bottom: rowBox.bottom - (swatchBox.bottom + reach),
      };
    });

    // A ring in the text colour is the heavy black (or white) outline.
    expect(indicator.shadows).not.toContain(indicator.textColor);
    expect(indicator.top).toBeGreaterThanOrEqual(8);
    expect(indicator.bottom).toBeGreaterThanOrEqual(8);
  });
}

for (const theme of ["light", "dark"] as const) {
  test(`toggle knobs look like the slider knob, on or off (${theme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: theme });
    await openSection(page, "General", `?theme=${theme}`);
    const toggles = page.getByRole("main").getByRole("checkbox");
    const knobs = await toggles.evaluateAll((inputs) =>
      inputs.map((input) => {
        const knob = getComputedStyle(input.nextElementSibling!, "::after");
        return {
          checked: (input as HTMLInputElement).checked,
          background: knob.backgroundColor,
          border: [
            knob.borderTopColor,
            knob.borderRightColor,
            knob.borderBottomColor,
            knob.borderLeftColor,
          ],
          // Tailwind puts empty ring layers in front of every shadow.
          shadow: knob.boxShadow
            .split("rgba(0, 0, 0, 0) 0px 0px 0px 0px, ")
            .join(""),
          transition: [knob.transitionProperty, knob.transitionDuration],
        };
      }),
    );

    expect(knobs.map((knob) => knob.checked)).toEqual(
      expect.arrayContaining([true, false]),
    );
    for (const knob of knobs) {
      expect(knob.background).toBe("rgb(255, 255, 255)");
      expect(knob.border).toEqual(Array(4).fill("rgba(0, 0, 0, 0.18)"));
      expect(knob.shadow).toBe("rgba(0, 0, 0, 0.25) 0px 1px 3px 0px");
      // The knob slides to its new side instead of jumping there.
      expect(knob.transition).toEqual(["translate", "0.2s"]);
    }
  });
}
