import { expect, test } from "@playwright/test";
import { MIN_WINDOW } from "./windowSizes";

const fixturePath = "/tests/fixtures/provider-onboarding.html";

const waitForProviderStep = async (page: import("@playwright/test").Page) => {
  await expect(
    page.getByRole("heading", { name: "Connect Codex" }),
  ).toBeVisible();
};

test("moves from granted permissions through a focus refresh to completion", async ({
  page,
}) => {
  await page.goto(fixturePath);
  await expect(page.getByTestId("provider-onboarding-mounted")).toBeAttached();
  await waitForProviderStep(page);

  await expect(
    page.getByText("Configuration not detected", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open Codex setup" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toBeDisabled();

  await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(true);
    window.dispatchEvent(new Event("focus"));
  });

  await expect(
    page.getByText("Configuration detected", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open Codex setup" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => window.providerOnboardingFixture.writes()))
    .toEqual(["complete"]);
  await expect
    .poll(() =>
      page.evaluate(() => window.providerOnboardingFixture.selectedLanguage()),
    )
    .toBe("fr");
});

test("keeps the provider step open when setup fails and routes transcription failures to recovery", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured`);
  await waitForProviderStep(page);

  await page.evaluate(() =>
    window.providerOnboardingFixture.failNextCompletion(),
  );
  await page.getByRole("button", { name: "Continue" }).click();
  await waitForProviderStep(page);
  await expect(
    page.getByText("Couldn't finish setup. Refresh the status and try again."),
  ).toBeVisible();
  await expect(
    page.getByText("Configuration changed before setup completed", {
      exact: true,
    }),
  ).toHaveCount(0);

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
  await expect(
    page.getByText("Configuration detected", { exact: true }),
  ).toBeVisible();

  await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(false);
    window.dispatchEvent(new Event("focus"));
  });
  await expect(
    page.getByText("Configuration not detected", { exact: true }),
  ).toBeVisible();
});

test("keeps French provider onboarding recoverable after completion result and IPC failures", async ({
  page,
}) => {
  const rawFailures = {
    exception: "Configuration changed before setup completed",
    result: "The provider status changed while completing setup",
  } as const;

  for (const mode of ["result", "exception"] as const) {
    await page.goto(`${fixturePath}?lang=fr&codex=configured`);
    const heading = page.getByRole("heading", { name: "Connecter Codex" });
    await expect(heading).toBeVisible();

    await page.evaluate(
      (failureMode) =>
        window.providerOnboardingFixture.failNextCompletion(failureMode),
      mode,
    );
    await page.getByRole("button", { name: "Continuer" }).click();

    await expect(heading).toBeVisible();
    await expect(
      page.getByText(
        "Impossible de terminer la configuration. Actualisez l’état et réessayez.",
      ),
    ).toBeVisible();
    await expect(
      page.getByText(rawFailures[mode], { exact: true }),
    ).toHaveCount(0);

    await page.getByRole("button", { name: "Continuer" }).click();
    await expect(page.getByRole("button", { name: "Général" })).toBeVisible();
  }
});

test("disables completion while refreshing, ignores stale statuses, and recovers after a status failure", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured`);
  await waitForProviderStep(page);
  const continueButton = page.getByRole("button", { name: "Continue" });
  await expect(continueButton).toBeEnabled();

  const initialRefresh = await page.evaluate(() => {
    const index = window.providerOnboardingFixture.deferStatusRefresh();
    window.dispatchEvent(new Event("focus"));
    return index;
  });
  await expect(continueButton).toBeDisabled();
  await page.evaluate(
    (index) => window.providerOnboardingFixture.resolveStatusRefresh(index),
    initialRefresh,
  );
  await expect(continueButton).toBeEnabled();

  const staleRefresh = await page.evaluate(() => {
    const index = window.providerOnboardingFixture.deferStatusRefresh();
    window.dispatchEvent(new Event("focus"));
    return index;
  });
  await expect(continueButton).toBeDisabled();
  const currentRefresh = await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(false);
    const index = window.providerOnboardingFixture.deferStatusRefresh();
    window.dispatchEvent(new Event("focus"));
    return index;
  });
  await page.evaluate(
    (index) => window.providerOnboardingFixture.resolveStatusRefresh(index),
    currentRefresh,
  );
  await expect(continueButton).toBeDisabled();
  await page.evaluate(
    (index) => window.providerOnboardingFixture.resolveStatusRefresh(index),
    staleRefresh,
  );
  await expect(continueButton).toBeDisabled();

  await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(true);
  });
  const failedRefresh = await page.evaluate(() => {
    const index = window.providerOnboardingFixture.deferStatusRefresh();
    window.dispatchEvent(new Event("focus"));
    return index;
  });
  await page.evaluate(
    (index) => window.providerOnboardingFixture.rejectStatusRefresh(index),
    failedRefresh,
  );
  await expect(
    page.getByText("Status unavailable", { exact: true }),
  ).toHaveCount(1);
  await expect(continueButton).toBeDisabled();
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(continueButton).toBeEnabled();
});

test("returns to permissions when post-onboarding initialization fails", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured`);
  await waitForProviderStep(page);

  await page.evaluate(() =>
    window.providerOnboardingFixture.failNextInitialization(),
  );
  await page.getByRole("button", { name: "Continue" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.providerOnboardingFixture
            .calls()
            .filter((command) => command === "initialize_enigo").length,
      ),
    )
    .toBeGreaterThanOrEqual(3);
  await expect
    .poll(() => page.evaluate(() => window.providerOnboardingFixture.writes()))
    .toContain("complete");
});

test("a returning user who grants a permission again goes straight back to the app", async ({
  page,
}) => {
  // Signed out of Codex: the provider step would have no way out.
  await page.goto(`${fixturePath}?returning=1`);
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Connect Codex" }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() => window.providerOnboardingFixture.calls()),
  ).toContain("show_main_window_command");
});

test("fits the French provider step at the native minimum window size", async ({
  page,
}) => {
  await page.setViewportSize(MIN_WINDOW);
  await page.goto(`${fixturePath}?lang=fr`);

  await expect(
    page.getByRole("heading", { name: "Connecter Codex" }),
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

test("localizes the French Codex setup failure and allows retry", async ({
  page,
}) => {
  const command = "plugin:opener|open_url";
  for (const mode of ["result", "exception"] as const) {
    await page.goto(`${fixturePath}?lang=fr`);
    await page.evaluate(
      (failureMode) =>
        window.providerOnboardingFixture.failNextSetupAction(failureMode),
      mode,
    );
    const action = page.getByRole("button", {
      name: "Ouvrir la configuration Codex",
      exact: true,
    });
    await action.click();
    await expect(
      page.getByText("Impossible d’ouvrir la configuration Codex.", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByText("The operating system refused to open this resource", {
        exact: true,
      }),
    ).toHaveCount(0);
    await action.click();
    await expect
      .poll(() =>
        page.evaluate(
          (name) =>
            window.providerOnboardingFixture
              .calls()
              .filter((call) => call === name).length,
          command,
        ),
      )
      .toBe(2);
    await expect(action).toBeEnabled();
  }
});
