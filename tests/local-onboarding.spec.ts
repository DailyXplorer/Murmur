import { expect, test, type Page } from "@playwright/test";
import { DEFAULT_WINDOW, MIN_WINDOW } from "./windowSizes";

const fixturePath = "/tests/fixtures/provider-onboarding.html";
const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

const fixture = (page: Page) => ({
  calls: () => page.evaluate(() => window.providerOnboardingFixture.calls()),
  engine: () => page.evaluate(() => window.providerOnboardingFixture.engine()),
  engines: () =>
    page.evaluate(() => window.providerOnboardingFixture.engines()),
  writes: () => page.evaluate(() => window.providerOnboardingFixture.writes()),
  setModel: (name: "ready" | "failed" | "downloading") =>
    page.evaluate(
      (model) => window.providerOnboardingFixture.setModel(model),
      name,
    ),
  connect: () =>
    page.evaluate(() => {
      window.providerOnboardingFixture.setCodexConfigured(true);
      window.dispatchEvent(new Event("focus"));
    }),
});

const modelStep = (page: Page) =>
  page.getByRole("heading", { name: "Transcribe on this Mac" });
const pill = (page: Page) =>
  page.getByRole("status").filter({ hasText: "Downloading the model… 38%" });

test("a new install downloads in the background, skips ChatGPT and waits for the model", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=missing`);
  const app = fixture(page);

  await expect(modelStep(page)).toBeVisible();
  await expect(
    page.getByText("It needs a one-time download of 505 MB.", { exact: false }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Download and continue" }).click();
  await expect.poll(app.engines).toEqual(["local"]);

  await expect(
    page.getByRole("heading", { name: "Connect ChatGPT (optional)" }),
  ).toBeVisible();
  await expect(pill(page)).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toHaveCount(0);
  await page.getByRole("button", { name: "Skip" }).click();

  await expect(page.getByRole("heading", { name: "You're set" })).toBeVisible();
  await expect(
    page.getByText(
      "The model is still downloading (38%). You can dictate as soon as it's ready.",
      { exact: true },
    ),
  ).toBeVisible();
  expect(await app.writes()).toEqual(["complete"]);
  expect(await app.engine()).toBe("local");
  // The step says to hold the shortcut, so the shortcut already works.
  await expect.poll(app.calls).toContain("initialize_shortcuts");

  await page.getByRole("button", { name: "Start using Murmur" }).click();
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(pill(page)).toBeVisible();

  await app.setModel("ready");
  await expect(pill(page)).toHaveCount(0);
});

test("the pill shows the download while permissions are granted", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=missing&permissions=missing`);
  await page.getByRole("button", { name: "Download and continue" }).click();

  await expect(
    page.getByRole("heading", { name: "Permissions Required" }),
  ).toBeVisible();
  await expect(pill(page)).toBeVisible();

  await fixture(page).setModel("ready");
  await expect(pill(page)).toHaveCount(0);
});

test("using the cloud instead makes the ChatGPT step required", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=missing`);
  const app = fixture(page);

  await page.getByRole("button", { name: "Use the cloud instead" }).click();
  await expect.poll(app.engines).toEqual(["cloud"]);

  await expect(
    page.getByRole("heading", { name: "Connect Codex" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Skip" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Continue" })).toHaveCount(0);
  await expect(page.getByText("Downloading the model")).toHaveCount(0);

  await app.connect();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  expect(await app.writes()).toEqual(["complete"]);
  expect(await app.engine()).toBe("cloud");
  expect(await app.calls()).not.toContain("download_local_model");
});

test("a Mac that cannot run the local model skips the model step", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=unsupported`);
  const app = fixture(page);

  await expect(
    page.getByRole("heading", { name: "Connect Codex" }),
  ).toBeVisible();
  await expect(modelStep(page)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Skip" })).toHaveCount(0);
  await expect.poll(app.engines).toEqual(["cloud"]);
});

test("a returning user with a missing permission sees only permissions", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?returning=1&model=missing`);
  const app = fixture(page);

  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(modelStep(page)).toHaveCount(0);
  const calls = await app.calls();
  expect(calls).toContain("show_main_window_command");
  expect(calls).not.toContain("change_transcription_engine_setting");
  expect(calls).not.toContain("complete_onboarding");
});

test("a ready model and a connected account finish at once", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=ready&codex=configured`);
  const app = fixture(page);

  await expect(
    page.getByText("The model is already on this Mac.", { exact: false }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(
    page.getByRole("heading", { name: "Connect ChatGPT (optional)" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Skip" })).toHaveCount(0);
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(
    page.getByText("Hold your shortcut and speak.", { exact: true }),
  ).toBeVisible();
  expect(await app.calls()).not.toContain("download_local_model");
  await page.getByRole("button", { name: "Start using Murmur" }).click();
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
});

test("with an account, the cloud fills in until the model is ready", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=missing&codex=configured`);
  await page.getByRole("button", { name: "Download and continue" }).click();
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(
    page.getByText(
      "The model is still downloading (38%). Until it's ready, Murmur uses the cloud.",
      { exact: true },
    ),
  ).toBeVisible();
});

test("a stopped download can be retried or traded for the cloud", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?model=missing`);
  const app = fixture(page);
  await page.getByRole("button", { name: "Download and continue" }).click();
  await page.getByRole("button", { name: "Skip" }).click();
  await app.setModel("failed");

  await expect(
    page.getByRole("heading", { name: "The download stopped" }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "No internet connection. The download will resume where it stopped.",
      { exact: true },
    ),
  ).toBeVisible();
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("heading", { name: "You're set" })).toBeVisible();
  expect(await app.calls()).toContain("download_local_model");

  await app.setModel("failed");
  await page.getByRole("button", { name: "Use the cloud instead" }).click();
  // No account: the cloud needs one before dictation can work.
  await expect(
    page.getByRole("heading", { name: "Connect Codex" }),
  ).toBeVisible();
  expect(await app.engine()).toBe("cloud");

  await app.connect();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
});

test("the French model and finish steps fit the native minimum window size", async ({
  page,
}) => {
  await page.setViewportSize(MIN_WINDOW);
  await page.goto(`${fixturePath}?lang=fr&model=missing`);

  const fits = (testId: string) =>
    page
      .getByTestId(testId)
      .evaluate((element) => element.scrollHeight <= element.clientHeight);

  await expect(
    page.getByRole("heading", { name: "Transcrire sur ce Mac" }),
  ).toBeVisible();
  await expect.poll(() => fits("local-model-onboarding")).toBe(true);

  await page.getByRole("button", { name: "Télécharger et continuer" }).click();
  await page.getByRole("button", { name: "Passer" }).click();
  await fixture(page).setModel("failed");
  await expect(
    page.getByRole("heading", { name: "Le téléchargement s'est arrêté" }),
  ).toBeVisible();
  await expect.poll(() => fits("done-onboarding")).toBe(true);
});

test.describe("onboarding screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");

  for (const [size, viewport, theme] of [
    ["default", DEFAULT_WINDOW, "light"],
    ["default", DEFAULT_WINDOW, "dark"],
    ["min", MIN_WINDOW, "light"],
  ] as const) {
    test(`every step (${size}, ${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize(viewport);
      const app = fixture(page);
      const shot = (page: Page, name: string) =>
        page.screenshot({
          path: `${outputDir}/onboarding-${name}-${size}-${theme}.png`,
        });

      await page.goto(`${fixturePath}?model=unsupported`);
      await expect(
        page.getByRole("heading", { name: "Connect Codex" }),
      ).toBeVisible();
      await shot(page, "unsupported-mac");

      await page.goto(`${fixturePath}?model=missing&mac=intel`);
      await expect(modelStep(page)).toBeVisible();
      await shot(page, "model-intel");

      await page.goto(`${fixturePath}?model=missing&permissions=missing`);
      await expect(modelStep(page)).toBeVisible();
      await shot(page, "model");
      await page.getByRole("button", { name: "Download and continue" }).click();
      await expect(pill(page)).toBeVisible();
      await shot(page, "permissions-downloading");

      await page.goto(`${fixturePath}?model=missing`);
      await page.getByRole("button", { name: "Download and continue" }).click();
      await expect(page.getByRole("button", { name: "Skip" })).toBeVisible();
      await shot(page, "chatgpt-optional");
      await page.getByRole("button", { name: "Skip" }).click();
      await expect(
        page.getByRole("heading", { name: "You're set" }),
      ).toBeVisible();
      await shot(page, "done-downloading-no-account");
      await app.setModel("failed");
      await expect(
        page.getByRole("heading", { name: "The download stopped" }),
      ).toBeVisible();
      await shot(page, "done-failed");
      await app.setModel("downloading");
      await page.getByRole("button", { name: "Start using Murmur" }).click();
      await expect(pill(page)).toBeVisible();
      await shot(page, "main-window-downloading");

      await page.goto(`${fixturePath}?model=missing&codex=configured`);
      await page.getByRole("button", { name: "Download and continue" }).click();
      await page.getByRole("button", { name: "Continue" }).click();
      await expect(
        page.getByRole("heading", { name: "You're set" }),
      ).toBeVisible();
      await shot(page, "done-downloading-account");
      await app.setModel("ready");
      await expect(
        page.getByText("Hold your shortcut and speak.", { exact: true }),
      ).toBeVisible();
      await shot(page, "done-ready");

      await page.goto(`${fixturePath}?model=missing`);
      await page.getByRole("button", { name: "Use the cloud instead" }).click();
      await expect(
        page.getByRole("heading", { name: "Connect Codex" }),
      ).toBeVisible();
      await shot(page, "chatgpt-required");
    });
  }

  for (const [size, viewport] of [
    ["default", DEFAULT_WINDOW],
    ["min", MIN_WINDOW],
  ] as const) {
    for (const theme of ["light", "dark"] as const) {
      test(`the style step (${size}, ${theme})`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: theme });
        await page.setViewportSize(viewport);
        for (const lang of ["en", "fr"]) {
          await page.goto(
            `${fixturePath}?lang=${lang}&codex=configured&cleanup=on`,
          );
          await page
            .getByTestId("provider-onboarding")
            .getByRole("button")
            .first()
            .click();
          const step = page.getByTestId("style-onboarding");
          await expect(step).toBeVisible();
          await page.waitForTimeout(300);
          await page.screenshot({
            path: `${outputDir}/onboarding-style-${lang}-${size}-${theme}.png`,
          });
        }
      });

      test(`the ChatGPT step (${size}, ${theme})`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: theme });
        await page.setViewportSize(viewport);
        const shot = (name: string) =>
          page.screenshot({
            path: `${outputDir}/chatgpt-${name}-${size}-${theme}.png`,
          });
        const retry = () =>
          page
            .getByTestId("provider-onboarding")
            .getByRole("button", { name: "Retry" })
            .click();

        await page.goto(`${fixturePath}?model=missing`);
        await page
          .getByRole("button", { name: "Download and continue" })
          .click();
        await expect(
          page.getByRole("heading", { name: "Connect ChatGPT (optional)" }),
        ).toBeVisible();
        await page.waitForTimeout(300);
        await shot("optional");
        await retry();
        await page.waitForTimeout(300);
        await shot("optional-retried");

        await page.goto(`${fixturePath}?model=unsupported`);
        await expect(
          page.getByRole("heading", { name: "Connect Codex" }),
        ).toBeVisible();
        await page.waitForTimeout(300);
        await shot("required");
        await retry();
        await page.waitForTimeout(300);
        await shot("required-retried");

        await page.goto(`${fixturePath}?model=unsupported&codex=configured`);
        await expect(
          page.getByRole("heading", { name: "Connect Codex" }),
        ).toBeVisible();
        await page.waitForTimeout(300);
        await shot("connected");
      });
    }
  }
});
