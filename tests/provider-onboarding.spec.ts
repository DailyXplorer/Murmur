import { expect, test } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { MIN_WINDOW } from "./windowSizes";

const fixturePath = "/tests/fixtures/provider-onboarding.html";
const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
const LOCALES = readdirSync(localesDir);
const translation = (locale: string) =>
  JSON.parse(readFileSync(`${localesDir}${locale}/translation.json`, "utf8"));

const waitForProviderStep = async (page: import("@playwright/test").Page) => {
  await expect(
    page.getByRole("heading", { name: "Connect ChatGPT", exact: true }),
  ).toBeVisible();
};

const calls = (page: import("@playwright/test").Page) =>
  page.evaluate(() => window.providerOnboardingFixture.calls());

test("Start signs in in the browser, and Continue appears once the session is saved", async ({
  page,
}) => {
  await page.goto(fixturePath);
  await waitForProviderStep(page);
  const step = page.getByTestId("provider-onboarding");
  const buttons = step.getByRole("button");
  const line = step.getByRole("status");

  await step.getByRole("button", { name: "Start" }).click();
  await expect(buttons).toHaveText(["Cancel", "Retry"]);
  await expect(line).toHaveText("Finish signing in in your browser.");
  expect(await calls(page)).toContain("sign_in_with_chatgpt");
  expect(await calls(page)).not.toContain("plugin:opener|open_url");

  await page.evaluate(() => window.providerOnboardingFixture.resolveSignIn());
  await expect(buttons).toHaveText(["Continue", "Retry"]);
  await expect(line).toHaveText("");
  await step.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => window.providerOnboardingFixture.writes()))
    .toEqual(["complete"]);
});

test("Start does not come back while the status is checked after a sign-in", async ({
  page,
}) => {
  await page.goto(fixturePath);
  await waitForProviderStep(page);
  const step = page.getByTestId("provider-onboarding");
  const buttons = step.getByRole("button");

  await step.getByRole("button", { name: "Start" }).click();
  await expect(buttons).toHaveText(["Cancel", "Retry"]);
  const refresh = await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(true);
    return window.providerOnboardingFixture.deferStatusRefresh();
  });
  const statusChecks = async () =>
    (await calls(page)).filter((call) => call === "get_codex_auth_status")
      .length;
  const checksBefore = await statusChecks();
  await page.evaluate(() => window.providerOnboardingFixture.resolveSignIn());
  await expect.poll(statusChecks).toBe(checksBefore + 1);
  await expect(buttons).toHaveText(["Cancel", "Retry"]);

  await page.evaluate(
    (index) => window.providerOnboardingFixture.resolveStatusRefresh(index),
    refresh,
  );
  await expect(buttons).toHaveText(["Continue", "Retry"]);
});

test("a failed sign-in says why and brings Start back", async ({ page }) => {
  await page.goto(fixturePath);
  await waitForProviderStep(page);
  const step = page.getByTestId("provider-onboarding");
  const buttons = step.getByRole("button");
  const line = step.getByRole("status");

  for (const [error, message] of [
    [
      "port_busy",
      "Another sign-in is already open, maybe in Codex. Finish or close it, then try again.",
    ],
    ["timed_out", "Sign-in timed out. Try again."],
    ["denied", "Sign-in was cancelled in the browser."],
    [
      "not_entitled",
      "Codex isn't enabled for this ChatGPT workspace. Ask your workspace admin.",
    ],
    ["network", "Couldn't reach OpenAI. Check your connection and try again."],
    ["rejected", "OpenAI didn't accept the sign-in. Try again."],
    [
      "save_failed",
      "Signed in, but the session couldn't be saved on this Mac.",
    ],
    ["browser_failed", "Couldn't open your browser."],
    ["exception", "Sign-in failed. Try again."],
  ] as const) {
    await step.getByRole("button", { name: "Start" }).click();
    await expect(buttons).toHaveText(["Cancel", "Retry"]);
    await page.evaluate(
      (error) => window.providerOnboardingFixture.failSignIn(error),
      error,
    );
    await expect(line, error).toHaveText(message);
    await expect(buttons, error).toHaveText(["Start", "Retry"]);
  }
  expect(
    (await calls(page)).filter((call) => call === "sign_in_with_chatgpt"),
  ).toHaveLength(9);
});

test("Cancel stops the sign-in under way", async ({ page }) => {
  await page.goto(fixturePath);
  await waitForProviderStep(page);
  const step = page.getByTestId("provider-onboarding");
  const buttons = step.getByRole("button");

  await step.getByRole("button", { name: "Start" }).click();
  await step.getByRole("button", { name: "Cancel" }).click();
  await expect(buttons).toHaveText(["Start", "Retry"]);
  await expect(step.getByRole("status")).toHaveText("");
  expect(await calls(page)).toContain("cancel_chatgpt_sign_in");

  await step.getByRole("button", { name: "Start" }).click();
  await expect(buttons).toHaveText(["Cancel", "Retry"]);
  await page.evaluate(() => window.providerOnboardingFixture.resolveSignIn());
  await expect(buttons).toHaveText(["Continue", "Retry"]);
});

test("moves from granted permissions through a focus refresh to completion", async ({
  page,
}) => {
  await page.goto(fixturePath);
  await expect(page.getByTestId("provider-onboarding-mounted")).toBeAttached();
  await waitForProviderStep(page);

  await expect(page.getByRole("button", { name: "Start" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toHaveCount(0);

  await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(true);
    window.dispatchEvent(new Event("focus"));
  });

  await expect(page.getByRole("button", { name: "Start" })).toHaveCount(0);
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
  await expect(page.getByText("Signed in", { exact: true })).toBeVisible();

  await page.evaluate(() => {
    window.providerOnboardingFixture.setCodexConfigured(false);
    window.dispatchEvent(new Event("focus"));
  });
  await expect(page.getByText("Not signed in", { exact: true })).toBeVisible();
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
    const heading = page.getByRole("heading", {
      name: translation("fr").onboarding.provider.title,
      exact: true,
    });
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

test("keeps Continue through a refresh, ignores stale statuses, and recovers after a status failure", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured`);
  await waitForProviderStep(page);
  const step = page.getByTestId("provider-onboarding");
  const continueButton = page.getByRole("button", { name: "Continue" });
  await expect(continueButton).toBeEnabled();

  const initialRefresh = await page.evaluate(() => {
    const index = window.providerOnboardingFixture.deferStatusRefresh();
    window.dispatchEvent(new Event("focus"));
    return index;
  });
  await expect(step.getByRole("button")).toHaveText(["Continue", "Retry"]);
  await expect(continueButton).toBeEnabled();
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
  await expect(step.getByRole("button")).toHaveText(["Start", "Retry"]);
  await page.evaluate(
    (index) => window.providerOnboardingFixture.resolveStatusRefresh(index),
    staleRefresh,
  );
  await expect(step.getByRole("button")).toHaveText(["Start", "Retry"]);

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
  await expect(continueButton).toHaveCount(0);
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(continueButton).toBeEnabled();
  await expect(
    page.getByText("Status unavailable", { exact: true }),
  ).toHaveCount(0);
});

test("a connected user never sees Start while the first check runs", async ({
  page,
}) => {
  await page.goto(`${fixturePath}?codex=configured&slowStatus=1`);
  await waitForProviderStep(page);
  const buttons = page.getByTestId("provider-onboarding").getByRole("button");
  await expect(buttons).toHaveCount(0);

  await page.evaluate(() =>
    window.providerOnboardingFixture.resolveStatusRefresh(0),
  );
  await expect(buttons).toHaveText(["Continue", "Retry"]);
});

test("Retry says it is checking until the answer comes back", async ({
  page,
}) => {
  await page.goto(fixturePath);
  await waitForProviderStep(page);
  const step = page.getByTestId("provider-onboarding");
  await step.getByRole("button", { name: "Retry" }).click();
  const line = step.getByRole("status");
  await expect(line).toHaveText("Not signed in");

  const refresh = await page.evaluate(() =>
    window.providerOnboardingFixture.deferStatusRefresh(),
  );
  await step.getByRole("button", { name: "Retry" }).click();
  await expect(line).toHaveText("Checking configuration…");
  await page.evaluate(
    (index) => window.providerOnboardingFixture.resolveStatusRefresh(index),
    refresh,
  );
  await expect(line).toHaveText("Not signed in");
});

for (const mode of [
  {
    name: "optional",
    query: "?model=missing",
    heading: "Connect ChatGPT (optional)",
    buttons: ["Start", "Retry", "Skip"],
  },
  {
    name: "required",
    query: "?model=unsupported",
    heading: "Connect ChatGPT",
    buttons: ["Start", "Retry"],
  },
]) {
  test(`the ${mode.name} ChatGPT step is a title, a description and one row of buttons`, async ({
    page,
  }) => {
    await page.goto(`${fixturePath}${mode.query}`);
    if (mode.name === "optional") {
      await page.getByRole("button", { name: "Download and continue" }).click();
    }
    await expect(
      page.getByRole("heading", { name: mode.heading, exact: true }),
    ).toBeVisible();
    const step = page.getByTestId("provider-onboarding");
    const buttons = step.getByRole("button");

    await expect(buttons).toHaveText(mode.buttons);
    await expect(step).not.toContainText("Codex app");
    await expect(step).not.toContainText("Not signed in");
    const expectOneEvenRow = async (when: string) => {
      const boxes = await buttons.evaluateAll((elements) =>
        elements.map((element) => {
          const box = element.getBoundingClientRect();
          return { width: Math.round(box.width), top: Math.round(box.top) };
        }),
      );
      expect(new Set(boxes.map((box) => box.width)).size, when).toBe(1);
      expect(new Set(boxes.map((box) => box.top)).size, when).toBe(1);
    };
    await expectOneEvenRow("before the sign-in");

    await step.getByRole("button", { name: "Start" }).click();
    await expect
      .poll(() => page.evaluate(() => window.providerOnboardingFixture.calls()))
      .toContain("sign_in_with_chatgpt");
    await expect(buttons).toHaveText(["Cancel", ...mode.buttons.slice(1)]);
    await expectOneEvenRow("while the browser is open");
    await step.getByRole("button", { name: "Cancel" }).click();

    await step.getByRole("button", { name: "Retry" }).click();
    const line = step.getByText("Not signed in", { exact: true });
    await expect(line).toBeVisible();
    const lineTop = (await line.boundingBox())!.y;
    const buttonsBottom = (await buttons.first().boundingBox())!;
    expect(lineTop, "the line sits under the buttons").toBeGreaterThanOrEqual(
      buttonsBottom.y + buttonsBottom.height,
    );

    await page.evaluate(() => {
      window.providerOnboardingFixture.setCodexConfigured(true);
      window.dispatchEvent(new Event("focus"));
    });
    await expect(buttons).toHaveText(["Continue", "Retry"]);
    await expect(line).toHaveCount(0);
    await expect(step.getByRole("button", { name: "Continue" })).toBeEnabled();
  });
}

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
  // Signed out of ChatGPT: the provider step would have no way out.
  await page.goto(`${fixturePath}?returning=1`);
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Connect ChatGPT", exact: true }),
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
  const fr = translation("fr");

  await expect(
    page.getByRole("heading", { name: fr.onboarding.provider.title }),
  ).toBeVisible();
  await expect(
    page.getByText(fr.settings.transcription.chatgpt.signedIn, {
      exact: true,
    }),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page
        .getByTestId("provider-onboarding")
        .evaluate((element) => element.scrollHeight <= element.clientHeight),
    )
    .toBe(true);
});

test("every locale keeps the ChatGPT buttons equal, on one row, at the minimum size", async ({
  page,
}) => {
  await page.setViewportSize(MIN_WINDOW);
  for (const locale of LOCALES) {
    await page.goto(`${fixturePath}?lang=${locale}&model=missing`);
    await page
      .getByTestId("local-model-onboarding")
      .getByRole("button")
      .first()
      .click();
    const { onboarding, settings, meetings } = translation(locale);
    const chatgpt = settings.transcription.chatgpt;
    const step = page.getByTestId("provider-onboarding");
    const buttons = step.getByRole("button");
    const line = step.getByRole("status");
    await expect(buttons, locale).toHaveText([
      onboarding.provider.start,
      onboarding.provider.retry,
      onboarding.provider.skip,
    ]);

    const expectFits = async (when: string) => {
      const layout = await step.evaluate((element) => ({
        fits: element.scrollHeight <= element.clientHeight,
        boxes: Array.from(element.querySelectorAll("button")).map((button) => {
          const box = button.getBoundingClientRect();
          return { width: Math.round(box.width), top: Math.round(box.top) };
        }),
      }));
      expect(layout.fits, `${locale} fits ${when}`).toBe(true);
      expect(
        new Set(layout.boxes.map((box) => box.width)).size,
        `${locale} equal widths ${when}`,
      ).toBe(1);
      expect(
        new Set(layout.boxes.map((box) => box.top)).size,
        `${locale} one row ${when}`,
      ).toBe(1);
    };

    await buttons.nth(1).click();
    await expect(line, locale).toHaveText(settings.transcription.missing);
    await expectFits("after Retry");

    await buttons.first().click();
    await expect(buttons, locale).toHaveText([
      meetings.cancel,
      onboarding.provider.retry,
      onboarding.provider.skip,
    ]);
    await expect(line, locale).toHaveText(chatgpt.waiting);
    await expectFits("while the browser is open");

    await page.evaluate(() =>
      window.providerOnboardingFixture.failSignIn("port_busy"),
    );
    await expect(line, locale).toHaveText(chatgpt.errors.portBusy);
    await expectFits("after a failure");
  }
});

test("localizes a failed sign-in in French and allows another try", async ({
  page,
}) => {
  const fr = translation("fr");
  for (const [error, key] of [
    ["port_busy", "portBusy"],
    ["exception", "generic"],
  ] as const) {
    await page.goto(`${fixturePath}?lang=fr`);
    const start = page.getByRole("button", {
      name: fr.onboarding.provider.start,
      exact: true,
    });
    await start.click();
    await page.evaluate(
      (error) => window.providerOnboardingFixture.failSignIn(error),
      error,
    );
    await expect(
      page.getByText(fr.settings.transcription.chatgpt.errors[key], {
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.getByText("IPC channel closed")).toHaveCount(0);
    await start.click();
    await expect
      .poll(async () =>
        (await calls(page)).filter((call) => call === "sign_in_with_chatgpt"),
      )
      .toHaveLength(2);
  }
});
