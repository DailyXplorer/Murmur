import { expect, test } from "@playwright/test";

const fixturePath = "/tests/fixtures/provider-onboarding.html";

const waitForProviderStep = async (page: import("@playwright/test").Page) => {
  await expect(
    page.getByRole("heading", { name: "Choose a transcription service" }),
  ).toBeVisible();
};

test("moves from granted permissions through a focus refresh to explicit provider completion", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?provider=gemini`);
  await expect(page.getByTestId("provider-onboarding-mounted")).toBeAttached();
  await waitForProviderStep(page);

  const codex = page.getByRole("radio", { name: "Codex" });
  await expect(codex).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Open Codex setup" }),
  ).toBeVisible();
  await expect(page.getByRole("radio", { name: "Antigravity" })).toBeChecked();
  await expect(page.getByRole("button", { name: "Install" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toBeDisabled();

  await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(true);
    window.dispatchEvent(new Event("focus"));
  });

  await expect(codex).toBeEnabled();
  await expect(
    page.getByText("Configuration detected", { exact: true }),
  ).toBeVisible();
  await codex.click();
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => window.providerOnboardingFixture.provider()),
    )
    .toBe("codex");
  await expect
    .poll(() => page.evaluate(() => window.providerOnboardingFixture.writes()))
    .toEqual(["provider:codex", "complete"]);
});

test("persists either visible provider choice before onboarding completes", async ({
  page,
}) => {
  await page.goto(
    `${fixturePath}?provider=gemini&codex=configured&gemini=configured`,
  );
  await waitForProviderStep(page);

  await page.getByRole("radio", { name: "Codex" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.providerOnboardingFixture.provider()),
    )
    .toBe("codex");

  await page.goto(
    `${fixturePath}?provider=codex&codex=configured&gemini=configured`,
  );
  await waitForProviderStep(page);
  await expect(page.getByRole("radio", { name: "Codex" })).toBeChecked();
  await page.getByRole("radio", { name: "Antigravity" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.providerOnboardingFixture.provider()),
    )
    .toBe("gemini");
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Auto Detect" }),
  ).toBeDisabled();
  await expect(
    page.getByText(
      "Antigravity detects the language automatically. Your Codex language selection is kept for when you switch back.",
    ),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => window.providerOnboardingFixture.selectedLanguage()),
    )
    .toBe("fr");
});

test("keeps the provider step open when setup fails and routes transcription failures to recovery", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured&gemini=configured`);
  await waitForProviderStep(page);

  await page.evaluate(() =>
    window.providerOnboardingFixture.failNextCompletion(),
  );
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(
    page.getByRole("heading", { name: "Choose a transcription service" }),
  ).toBeVisible();
  await expect(
    page.getByText("Couldn't finish setup. Refresh the status and try again."),
  ).toBeVisible();

  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await page.evaluate(() =>
    window.providerOnboardingFixture.emitTranscriptionFailure(),
  );
  await page
    .getByRole("button", { name: "Review transcription service" })
    .click();
  await expect(
    page.getByRole("button", { name: "Transcription", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();

  await page.evaluate(() =>
    window.providerOnboardingFixture.failNextProviderChange(),
  );
  await page.getByRole("button", { name: "Codex", exact: true }).click();
  await page.getByRole("option", { name: "Codex" }).click();
  await expect(
    page.getByText(
      "Couldn't save the selected transcription service. Please retry.",
    ),
  ).toBeVisible();
});

test("fits the French provider choice at the native minimum window size", async ({
  page,
}) => {
  await page.setViewportSize({ width: 680, height: 570 });
  await page.goto(`${fixturePath}?provider=gemini&lang=fr`);

  await expect(
    page.getByRole("heading", { name: "Choisir un service de transcription" }),
  ).toBeVisible();
  await expect(
    page.getByText("Configuration détectée", { exact: true }),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page
        .getByTestId("provider-onboarding")
        .evaluate((element) => element.scrollHeight <= element.clientHeight),
    )
    .toBe(true);
});
