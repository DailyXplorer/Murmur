import { expect, test, type Page } from "@playwright/test";
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { MIN_WINDOW } from "./windowSizes";

const LOCALES = readdirSync(
  fileURLToPath(new URL("../src/i18n/locales/", import.meta.url)),
);
const fixturePath = "/tests/fixtures/provider-onboarding.html";

const EXAMPLES = [
  "I think we can meet tomorrow around 3 p.m., if you are available. Otherwise, Thursday also works.",
  "I think we can meet tomorrow around 3, if you're free. Otherwise, Thursday works too.",
  "I think we can meet tomorrow around 3 if you're free, otherwise Thursday works too",
];

const style = (page: Page, name: string) =>
  page
    .getByRole("radiogroup", { name: "Dictation style" })
    .getByRole("radio", { name: new RegExp(`^${name}`) });
const styleStep = (page: Page) =>
  page.getByRole("heading", { name: "Choose a style" });
const fixture = (page: Page) => ({
  calls: () => page.evaluate(() => window.providerOnboardingFixture.calls()),
  style: () => page.evaluate(() => window.providerOnboardingFixture.style()),
});

test("follows a connected account when cleanup can run, and Continue saves the pick", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured&cleanup=on`);
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(styleStep(page)).toBeVisible();
  for (const example of EXAMPLES) {
    await expect(page.getByText(example, { exact: true })).toBeVisible();
  }
  await expect(style(page, "Normal")).toBeChecked();
  await style(page, "Casual").check();
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  expect(await fixture(page).style()).toBe("casual");
});

test("preselects the stored style, and Skip keeps it", async ({ page }) => {
  await page.goto(`${fixturePath}?codex=configured&cleanup=on`);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(style(page, "Normal")).toBeChecked();
  await style(page, "Formal").check();
  await page.getByRole("button", { name: "Skip" }).click();

  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  expect(await fixture(page).style()).toBe("normal");
  expect(await fixture(page).calls()).not.toContain(
    "change_dictation_style_setting",
  );

  await page.goto(`${fixturePath}?codex=configured&cleanup=on&style=formal`);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(style(page, "Formal")).toBeChecked();
});

test("on this Mac, the style step comes before the finish step", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=ready&codex=configured&cleanup=on`);
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(styleStep(page)).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("heading", { name: "You're set" })).toBeVisible();
});

test("is skipped when AI cleanup is off", async ({ page }) => {
  await page.goto(`${fixturePath}?codex=configured`);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(styleStep(page)).toHaveCount(0);
});

test("is skipped on this Mac when the account step was skipped", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=missing&cleanup=on`);
  await page.getByRole("button", { name: "Download and continue" }).click();
  await page.getByRole("button", { name: "Skip" }).click();
  await expect(page.getByRole("heading", { name: "You're set" })).toBeVisible();
  await expect(styleStep(page)).toHaveCount(0);
});

test("never shows for a returning user granting a permission again", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?returning=1&codex=configured&cleanup=on`);
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(styleStep(page)).toHaveCount(0);
});

test("every locale fits the style step at the minimum window size", async ({
  page,
}) => {
  await page.setViewportSize(MIN_WINDOW);
  for (const locale of LOCALES) {
    await page.goto(
      `${fixturePath}?lang=${locale}&codex=configured&cleanup=on`,
    );
    await page
      .getByTestId("provider-onboarding")
      .getByRole("button")
      .first()
      .click();
    const step = page.getByTestId("style-onboarding");
    await expect(step.getByRole("radio"), locale).toHaveCount(3);
    expect(
      await step.evaluate(
        (element) => element.scrollHeight <= element.clientHeight,
      ),
      `${locale} fits`,
    ).toBe(true);
  }
});
