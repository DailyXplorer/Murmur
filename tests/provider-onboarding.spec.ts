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
  const automaticLanguageDescription =
    "Antigravity detects the language automatically. Your Codex language selection is kept for when you switch back.";
  const automaticLanguageInfo = page.getByRole("button", {
    name: automaticLanguageDescription,
  });
  await expect(automaticLanguageInfo).toBeVisible();
  await expect(
    page.getByText(automaticLanguageDescription, { exact: true }),
  ).toHaveCount(0);
  await automaticLanguageInfo.hover();
  await expect(page.getByRole("tooltip")).toHaveText(
    automaticLanguageDescription,
  );
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
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(0);

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

  await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(false);
    window.dispatchEvent(new Event("focus"));
  });
  await expect(
    page.getByText("Configuration not detected", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Codex", exact: true }).click();
  await expect(page.getByRole("option", { name: "Codex" })).toBeDisabled();
  await expect(page.getByRole("option", { name: "Antigravity" })).toBeEnabled();
});

test("keeps French provider onboarding recoverable after completion result and IPC failures", async ({
  page,
}) => {
  const rawFailures = {
    exception: "Configuration changed before setup completed",
    result: "The provider status changed while completing setup",
  } as const;

  for (const mode of ["result", "exception"] as const) {
    await page.goto(
      `${fixturePath}?lang=fr&provider=gemini&codex=configured&gemini=configured`,
    );
    await expect(
      page.getByRole("heading", {
        name: "Choisir un service de transcription",
      }),
    ).toBeVisible();

    const antigravity = page.getByRole("radio", { name: "Antigravity" });
    await expect(antigravity).toBeChecked();
    await page.evaluate(
      (failureMode) =>
        window.providerOnboardingFixture.failNextCompletion(failureMode),
      mode,
    );
    await page.getByRole("button", { name: "Continuer" }).click();

    await expect(
      page.getByRole("heading", {
        name: "Choisir un service de transcription",
      }),
    ).toBeVisible();
    await expect(antigravity).toBeChecked();
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
    await expect
      .poll(() =>
        page.evaluate(() => window.providerOnboardingFixture.provider()),
      )
      .toBe("gemini");
  }
});

test("disables completion while refreshing, ignores stale statuses, and recovers after a status failure", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured&gemini=configured`);
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
  ).toHaveCount(2);
  await expect(continueButton).toBeDisabled();
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(continueButton).toBeEnabled();
});

test("preserves the persisted Antigravity choice when post-onboarding initialization fails", async ({
  page,
}) => {
  await page.goto(
    `${fixturePath}?provider=gemini&codex=configured&gemini=configured`,
  );
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
  await expect(page.getByRole("radio", { name: "Antigravity" })).toBeChecked();
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

for (const view of ["onboarding", "settings"] as const) {
  test(`localizes French setup action failures in ${view} and allows retry`, async ({
    page,
  }) => {
    const scenarios = [
      {
        query: "gemini=installed&codex=configured",
        button: "Ouvrir Antigravity",
        message: "Impossible d’ouvrir Antigravity.",
        command: "open_antigravity",
      },
      {
        query: "codex=configured",
        button: "Installer",
        message: "Impossible d’ouvrir la page de téléchargement d’Antigravity.",
        command: "plugin:opener|open_url",
      },
      ...(view === "onboarding"
        ? [
            {
              query: "gemini=configured",
              button: "Ouvrir la configuration Codex",
              message: "Impossible d’ouvrir la configuration Codex.",
              command: "plugin:opener|open_url",
            },
          ]
        : []),
    ];
    for (const scenario of scenarios) {
      for (const mode of ["result", "exception"] as const) {
        await page.goto(`${fixturePath}?lang=fr&${scenario.query}`);
        if (view === "settings") {
          await page.getByRole("button", { name: "Continuer" }).click();
          await page
            .getByRole("button", { name: "Transcription", exact: true })
            .click();
        }
        await page.evaluate(
          (failureMode) =>
            window.providerOnboardingFixture.failNextSetupAction(failureMode),
          mode,
        );
        const action = page.getByRole("button", {
          name: scenario.button,
          exact: true,
        });
        await action.click();
        await expect(
          page.getByText(scenario.message, { exact: true }),
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
              (command) =>
                window.providerOnboardingFixture
                  .calls()
                  .filter((call) => call === command).length,
              scenario.command,
            ),
          )
          .toBe(2);
        await expect(action).toBeEnabled();
      }
    }
  });
}
