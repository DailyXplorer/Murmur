import { expect, test, type Page } from "@playwright/test";

const open = async (page: Page, query: string, section: string) => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: section, exact: true })
    .click();
};

const callsTo = (page: Page, command: string) =>
  page.evaluate(
    (name) =>
      window.settingsPages.calls
        .filter((call) => call.command === name)
        .map((call) => call.args ?? null),
    command,
  );

const expectNoUnhandledCommands = async (page: Page) =>
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);

test.describe("how Murmur transcribes", () => {
  test("the choice switches the engine", async ({ page }) => {
    await open(page, "engine=cloud", "Transcription");
    const local = page.getByRole("radio", { name: /On This Mac/ });
    const cloud = page.getByRole("radio", { name: /In the Cloud/ });
    await expect(cloud).toBeChecked();
    await expect(page.getByText("Parakeet v3")).toHaveCount(0);

    await local.check();
    await expect(local).toBeChecked();
    await expect(page.getByText("Parakeet v3")).toBeVisible();
    await expect(page.getByText("Ready · 505 MB")).toBeVisible();

    await cloud.check();
    await expect(cloud).toBeChecked();
    await expect(page.getByText("Parakeet v3")).toHaveCount(0);
    expect(await callsTo(page, "change_transcription_engine_setting")).toEqual([
      { engine: "local" },
      { engine: "cloud" },
    ]);
    await expectNoUnhandledCommands(page);
  });

  test("picking this Mac without the model starts its download", async ({
    page,
  }) => {
    await open(page, "engine=cloud&models=missing", "Transcription");
    await page.getByRole("radio", { name: /On This Mac/ }).check();
    await expect(page.getByText("Downloading… 212 of 505 MB")).toBeVisible();
    await expect(page.getByRole("progressbar")).toHaveAttribute(
      "aria-valuenow",
      "42",
    );
    expect(await callsTo(page, "change_transcription_engine_setting")).toEqual([
      { engine: "local" },
    ]);
    await expectNoUnhandledCommands(page);
  });

  test("a refused engine change says so and keeps the engine", async ({
    page,
  }) => {
    await open(page, "engine=cloud&engineRefused=1", "Transcription");
    await page.getByRole("radio", { name: /On This Mac/ }).click();
    await expect(
      page.getByText("Couldn't change how Murmur transcribes."),
    ).toBeVisible();
    await expect(
      page.getByRole("radio", { name: /In the Cloud/ }),
    ).toBeChecked();
    await expectNoUnhandledCommands(page);
  });

  test("a Mac that cannot run the model keeps the cloud", async ({ page }) => {
    await open(page, "engine=cloud&analysis=os_too_old", "Transcription");
    await expect(
      page.getByRole("radio", { name: /On This Mac/ }),
    ).toBeDisabled();
    await expect(
      page.getByText("Local transcription needs macOS 15 or later."),
    ).toBeVisible();
    await expectNoUnhandledCommands(page);
  });

  for (const [models, text, button] of [
    ["missing", "Not downloaded", "Download (505 MB)"],
    ["downloading", "Downloading… 212 of 505 MB", "Cancel"],
    ["preparing", "Preparing for this Mac…", null],
    [
      "failed",
      "No internet connection. The download will resume where it stopped.",
      "Try Again",
    ],
    ["damaged", "The downloaded file is damaged. Try again.", "Try Again"],
    ["noSpace", "Not enough space: 556 MB needed, 120 MB free.", "Try Again"],
    ["ready", "Ready · 505 MB", "Delete"],
  ] as const) {
    test(`the model row when ${models}`, async ({ page }) => {
      await open(page, `engine=local&models=${models}`, "Transcription");
      await expect(page.getByText(text, { exact: false })).toBeVisible();
      const actions = page
        .getByTestId("local-model-row")
        .getByRole("button", { name: /^(Download|Cancel|Try Again|Delete)/ });
      if (button) await expect(actions).toHaveText([button]);
      else await expect(actions).toHaveCount(0);
      await expectNoUnhandledCommands(page);
    });
  }

  test("the model row's buttons call their commands", async ({ page }) => {
    await open(page, "engine=local&models=failed", "Transcription");
    await page.getByRole("button", { name: "Try Again" }).click();
    await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByText("Not downloaded")).toBeVisible();
    expect(await callsTo(page, "download_local_model")).toHaveLength(1);
    expect(await callsTo(page, "cancel_local_model_download")).toHaveLength(1);
    await expectNoUnhandledCommands(page);
  });

  for (const [query, expected, absent] of [
    [
      "",
      [
        "Dictation will use the cloud until you download it again.",
        "Meetings will show one speaker per audio source.",
      ],
      "You won't be able to dictate",
    ],
    [
      "&account=none",
      [
        "You won't be able to dictate until you download it again or connect ChatGPT.",
        "Meetings will show one speaker per audio source.",
      ],
      "Dictation will use the cloud",
    ],
    [
      "&analysis=unsupported",
      ["Dictation will use the cloud until you download it again."],
      "Meetings will show one speaker",
    ],
  ] as const) {
    test(`delete asks first (${query || "with an account"})`, async ({
      page,
    }) => {
      await open(page, `engine=local${query}`, "Transcription");
      await page.getByRole("button", { name: "Delete" }).click();
      const dialog = page.getByRole("dialog", {
        name: "Delete the Local Model?",
      });
      for (const text of expected) {
        await expect(dialog.getByText(text)).toBeVisible();
      }
      await expect(dialog.getByText(absent, { exact: false })).toHaveCount(0);
      expect(await callsTo(page, "delete_local_model")).toHaveLength(0);

      await dialog.getByRole("button", { name: "Delete Model" }).click();
      await expect(dialog).toHaveCount(0);
      await expect(page.getByText("Not downloaded")).toBeVisible();
      expect(await callsTo(page, "delete_local_model")).toHaveLength(1);
      await expectNoUnhandledCommands(page);
    });
  }

  test("without ChatGPT, local mode offers it and AI cleanup waits for it", async ({
    page,
  }) => {
    await open(page, "engine=local&account=none", "Transcription");
    await expect(
      page.getByText("Optional: connect for AI cleanup and more languages."),
    ).toBeVisible();
    await expect(
      page.getByRole("checkbox", { name: "AI Cleanup and Formatting" }),
    ).toBeDisabled();
    await expect(
      page.getByText("Connect ChatGPT to use AI cleanup."),
    ).toBeVisible();
    await expectNoUnhandledCommands(page);
  });

  test("with ChatGPT, AI cleanup stays available", async ({ page }) => {
    await open(page, "engine=local", "Transcription");
    await expect(
      page.getByRole("checkbox", { name: "AI Cleanup and Formatting" }),
    ).toBeEnabled();
    await expect(
      page.getByText("Connect ChatGPT to use AI cleanup."),
    ).toHaveCount(0);
    await expect(
      page.getByText("Optional: connect for AI cleanup and more languages."),
    ).toHaveCount(0);
  });
});

test.describe("the one-time local transcription card", () => {
  const card = (page: Page) =>
    page.getByRole("region", { name: "New: local transcription, no quota." });

  test("Download switches to this Mac once the model is ready", async ({
    page,
  }) => {
    await open(page, "card=1&models=missing", "General");
    await card(page).getByRole("button", { name: "Download (505 MB)" }).click();
    await expect(card(page).getByText(/Downloading… 42%/)).toBeVisible();
    expect(await callsTo(page, "switch_to_local_when_ready")).toHaveLength(1);

    await page.evaluate(() => window.settingsPages.finishModelDownload());
    await expect(page.getByText("Now transcribing on this Mac.")).toBeVisible();
    await expect(card(page)).toHaveCount(0);

    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Transcription", exact: true })
      .click();
    await expect(
      page.getByRole("radio", { name: /On This Mac/ }),
    ).toBeChecked();
    await expectNoUnhandledCommands(page);
  });

  test("a model already on this Mac switches at once", async ({ page }) => {
    await open(page, "card=1&models=ready", "Transcription");
    await card(page)
      .getByRole("button", { name: "Use Local Transcription" })
      .click();
    await expect(page.getByText("Now transcribing on this Mac.")).toBeVisible();
    await expect(card(page)).toHaveCount(0);
    await expect(
      page.getByRole("radio", { name: /On This Mac/ }),
    ).toBeChecked();
    await expectNoUnhandledCommands(page);
  });

  test("closing it saves that it was closed", async ({ page }) => {
    await open(page, "card=1", "General");
    await card(page).getByRole("button", { name: "Dismiss" }).click();
    await expect(card(page)).toHaveCount(0);
    expect(
      await callsTo(page, "change_local_stt_card_dismissed_setting"),
    ).toEqual([{ dismissed: true }]);
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Transcription", exact: true })
      .click();
    await expect(card(page)).toHaveCount(0);
    await expectNoUnhandledCommands(page);
  });

  for (const [query, section] of [
    ["card=1&engine=local", "General"],
    ["card=1&analysis=os_too_old", "General"],
    ["card=1", "History"],
    ["", "General"],
  ] as const) {
    test(`it stays hidden (${query || "dismissed"}, ${section})`, async ({
      page,
    }) => {
      await open(page, query, section);
      await expect(page.getByRole("main", { name: section })).toBeVisible();
      await expect
        .poll(() => callsTo(page, "get_local_model_status"))
        .not.toHaveLength(0);
      await page.waitForTimeout(300);
      await expect(card(page)).toHaveCount(0);
      await expectNoUnhandledCommands(page);
    });
  }

  const cardDownloadThen = async (page: Page, undo: () => Promise<void>) => {
    await open(page, "card=1&models=missing", "General");
    await card(page).getByRole("button", { name: "Download (505 MB)" }).click();
    await expect(card(page).getByText(/Downloading… 42%/)).toBeVisible();
    await undo();
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "General", exact: true })
      .click();
    await expect(card(page)).toBeVisible();
    await expect(card(page).getByText(/Murmur switches/)).toHaveCount(0);
  };

  test("a download Meetings starts after a cancel switches nothing", async ({
    page,
  }) => {
    await cardDownloadThen(page, async () => {
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.getByRole("button", { name: "Cancel" }).click();
      await page.getByRole("button", { name: "Download", exact: true }).click();
      await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
    });
    await expect(
      card(page).getByRole("button", { name: "Use Local Transcription" }),
    ).toBeVisible();
    await expectNoUnhandledCommands(page);
  });

  test("choosing the cloud by hand drops the pending switch", async ({
    page,
  }) => {
    await cardDownloadThen(page, async () => {
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Transcription", exact: true })
        .click();
      await page.getByRole("radio", { name: /On This Mac/ }).check();
      await page.getByRole("radio", { name: /In the Cloud/ }).check();
      await expect(
        page.getByRole("radio", { name: /In the Cloud/ }),
      ).toBeChecked();
    });
    await expect(
      card(page).getByRole("button", { name: "Use Local Transcription" }),
    ).toBeVisible();
    await expectNoUnhandledCommands(page);
  });

  test("it shows on General and Transcription", async ({ page }) => {
    for (const section of ["General", "Transcription"]) {
      await open(page, "card=1", section);
      await expect(card(page)).toBeVisible();
    }
  });
});

test.describe("lines about the engine used", () => {
  test("History marks only the fallbacks", async ({ page }) => {
    await open(page, "history=fallback", "History");
    for (const text of [
      "Transcribed in the cloud: the local model wasn't ready.",
      "Transcribed in the cloud: local transcription failed.",
      "Transcribed in the cloud: this language isn't supported on this Mac.",
      "Transcribed on this Mac: the ChatGPT limit was reached.",
      "Transcribed on this Mac: ChatGPT was unavailable.",
    ]) {
      await expect(page.getByText(text)).toHaveCount(1);
    }
    await expect(
      page.getByText(/^Transcribed (in the cloud|on this Mac):/),
    ).toHaveCount(5);
    await expectNoUnhandledCommands(page);
  });

  test("History shows no line for the chosen engine", async ({ page }) => {
    await open(page, "", "History");
    await expect(
      page.getByText("The quarterly report is ready for review."),
    ).toBeVisible();
    await expect(
      page.getByText(/^Transcribed (in the cloud|on this Mac):/),
    ).toHaveCount(0);
  });

  for (const [query, count] of [
    ["&localFallback=1", 1],
    ["", 0],
  ] as const) {
    test(`a meeting says when this Mac filled in (${count})`, async ({
      page,
    }) => {
      await open(page, `meetings=1${query}`, "Meetings");
      await page.getByRole("button", { name: /Point hebdo produit/ }).click();
      await expect(page.getByRole("tab", { name: "Summary" })).toBeVisible();
      await expect(
        page.getByText(
          "Codex was unavailable for 12 minutes of this meeting, so that part was transcribed on this Mac.",
        ),
      ).toHaveCount(count);
      await expectNoUnhandledCommands(page);
    });
  }

  test("About credits NVIDIA Parakeet", async ({ page }) => {
    await open(page, "", "About");
    await expect(
      page.getByText("Local transcription uses NVIDIA Parakeet (CC BY 4.0)."),
    ).toBeVisible();
    await page.getByRole("button", { name: "View Licences" }).click();
    const dialog = page.getByRole("dialog", { name: "Model Licences" });
    await expect(
      dialog.getByText(/NVIDIA Parakeet TDT 0\.6B v3/),
    ).toBeVisible();
    await expect(dialog.getByText(/CC BY 4\.0/).first()).toBeVisible();
    await expect(dialog.getByText(/handy-computer/)).toBeVisible();
    await expectNoUnhandledCommands(page);
  });
});
