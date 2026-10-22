import { expect, test, type Page } from "@playwright/test";
import { DEFAULT_WINDOW } from "./windowSizes";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

const openTranscription = async (page: Page, query: string) => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
  const section = page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true });
  await section.click();
  await expect(section).toHaveAttribute("aria-current", "page");
  await page.waitForTimeout(500);
};

test.describe("ChatGPT sign-in screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ viewport: DEFAULT_WINDOW });

  for (const theme of ["light", "dark"] as const) {
    const shot = (page: Page, name: string) =>
      page.screenshot({ path: `${outputDir}/${name}-${theme}.png` });

    test(`onboarding (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await page.goto(
        `/tests/fixtures/provider-onboarding.html?theme=${theme}&model=unsupported`,
      );
      const step = page.getByTestId("provider-onboarding");
      const line = step.getByRole("status");
      await expect(step.getByRole("button", { name: "Start" })).toBeVisible();
      await shot(page, "onboarding-idle");

      await step.getByRole("button", { name: "Start" }).click();
      await expect(line).toHaveText("Finish signing in in your browser.");
      await shot(page, "onboarding-waiting");

      await page.evaluate(() =>
        window.providerOnboardingFixture.failSignIn("port_busy"),
      );
      await expect(line).toHaveText(/^Another sign-in is already open/);
      await shot(page, "onboarding-error");

      await step.getByRole("button", { name: "Start" }).click();
      await page.evaluate(() =>
        window.providerOnboardingFixture.resolveSignIn(),
      );
      await expect(
        step.getByRole("button", { name: "Continue" }),
      ).toBeVisible();
      await shot(page, "onboarding-connected");
    });

    test(`settings (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      for (const [name, account, button] of [
        ["settings-signed-in", "signedIn", "Sign out"],
        ["settings-signed-out", "none", "Sign in"],
      ] as const) {
        await openTranscription(page, `theme=${theme}&account=${account}`);
        await expect(
          page.getByRole("button", { name: button, exact: true }),
        ).toBeVisible();
        await shot(page, name);
      }

      await openTranscription(page, `theme=${theme}`);
      await page.getByRole("button", { name: "Sign out", exact: true }).click();
      await expect(
        page.getByRole("dialog", { name: "Sign out of ChatGPT?" }),
      ).toBeVisible();
      await page.waitForTimeout(300);
      await shot(page, "sign-out-confirm");
      expect(
        await page.evaluate(() => window.settingsPages.unhandledCommands),
      ).toEqual([]);
    });
  }
});
