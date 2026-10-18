import { expect, test, type Locator, type Page } from "@playwright/test";

// The window opens at this size (lib.rs) and cannot be made smaller.
const DEFAULT_WINDOW = { width: 680, height: 570 };

const SECTIONS = [
  "General",
  "Transcription",
  "History",
  "Meetings",
  "Insights",
];

const openSection = async (page: Page, section: string, query = "") => {
  await page.setViewportSize(DEFAULT_WINDOW);
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

const GEMINI_STATUS = {
  notInstalled: "Antigravity not installed",
  notSignedIn: "Configuration not detected",
  ready: "Configuration detected",
};

for (const [gemini, statusText] of Object.entries(GEMINI_STATUS)) {
  test(`Antigravity row stacks its status under the title (${gemini})`, async ({
    page,
  }) => {
    await openSection(page, "Transcription", `?gemini=${gemini}`);
    const row = settingRow(page, "Antigravity");
    const title = await box(row.getByRole("heading"));
    const badge = await box(row.getByText("Experimental", { exact: true }));
    const status = row.getByText(statusText, { exact: true });

    expect(
      Math.abs(badge.y + badge.height / 2 - (title.y + title.height / 2)),
    ).toBeLessThan(2);
    expect(badge.x).toBeGreaterThan(title.x + title.width);
    expect((await box(status)).y).toBeGreaterThanOrEqual(
      title.y + title.height,
    );
    expect(await isTruncated(status)).toBe(false);

    const action = row.getByRole("button", {
      name: /Install|Open Antigravity/,
    });
    if (gemini === "ready") {
      await expect(action).toHaveCount(0);
    } else {
      const button = await box(action);
      const statusBox = await box(status);
      expect(button.y).toBeGreaterThanOrEqual(title.y + title.height);
      expect(
        Math.abs(
          button.y + button.height / 2 - (statusBox.y + statusBox.height / 2),
        ),
      ).toBeLessThan(2);
    }
  });
}

test("Custom Words puts a full-width field under its title", async ({
  page,
}) => {
  await openSection(page, "Transcription");
  const row = settingRow(page, "Custom Words");
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

for (const language of ["en", "de", "es", "fr", "ru", "uk"]) {
  test(`Insights stat labels fit in full (${language})`, async ({ page }) => {
    await openSection(page, "Insights", `?lang=${language}`);
    // The stat cards are the sections that show a value right under the label.
    const tiles = page.getByRole("main").locator("section:has(> p)");
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
    .getByRole("button", { name: "Search Summary" })
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
    const row = settingRow(page, "Accent Color");
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
          shadow: knob.boxShadow.replaceAll(
            "rgba(0, 0, 0, 0) 0px 0px 0px 0px, ",
            "",
          ),
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
