import { expect, test, type Locator, type Page } from "@playwright/test";

const fixture = "/tests/fixtures/settings-pages.html?meetings=1";

const openMeeting = async (page: Page, title: RegExp, query = "") => {
  await page.goto(`${fixture}${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Meetings", exact: true })
    .click();
  await page.getByRole("button", { name: title }).click();
};

/** Opens the ready meeting on its Transcript tab. */
const openTranscript = async (page: Page, query = "") => {
  await openMeeting(page, /Point hebdo produit/, query);
  await page.getByRole("tab", { name: "Transcript" }).click();
};

const summary = (page: Page) => page.getByRole("region", { name: "Summary" });

const callsTo = (page: Page, command: string) =>
  page.evaluate(
    (name) =>
      window.settingsPages.calls
        .filter((call) => call.command === name)
        .map((call) => call.args),
    command,
  );

const TRANSPARENT = "rgba(0, 0, 0, 0)";

const cssOf = (locator: Locator, property: string) =>
  locator.evaluate(
    (element, name) => getComputedStyle(element).getPropertyValue(name),
    property,
  );

/** One line of text as small as the date chip's, not a card. */
const expectOneSmallLine = async (page: Page, notice: Locator) => {
  const chip = page.getByText(/Oct \d+/);
  await expect(notice).toHaveCSS("font-size", await cssOf(chip, "font-size"));
  await expect(notice).toHaveCSS("border-top-width", "0px");
  const lineHeight = parseFloat(await cssOf(notice, "line-height"));
  const box = await notice.boundingBox();
  expect(box?.height).toBeLessThanOrEqual(lineHeight + 1);
};

/** The buttons, in this order, are squares smaller than a form control, a
 * few pixels apart. */
const expectToolbar = async (page: Page, labels: string[]) => {
  const boxes = await Promise.all(
    labels.map((label) =>
      page.getByRole("button", { name: label, exact: true }).boundingBox(),
    ),
  );
  for (const box of boxes) {
    expect(box?.width).toBe(box?.height);
    expect(box?.height).toBeGreaterThanOrEqual(24);
    expect(box?.height).toBeLessThanOrEqual(30);
  }
  for (let index = 1; index < boxes.length; index++) {
    const gap =
      (boxes[index]?.x ?? 0) -
      ((boxes[index - 1]?.x ?? 0) + (boxes[index - 1]?.width ?? 0));
    expect(gap).toBeGreaterThan(0);
    expect(gap).toBeLessThanOrEqual(4);
  }
};

const bubbles = (page: Page) =>
  page.getByRole("list", { name: "Transcript" }).getByRole("listitem");

test("a meeting whose system audio was pure silence points to the privacy settings", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);
  await expect(page.getByText(/No system audio was recorded/)).toHaveCount(0);

  await openMeeting(
    page,
    /Point hebdo produit/,
    "&systemAudioSilent=1&systemAudioPermission=denied",
  );
  const notice = page
    .getByRole("status")
    .filter({ hasText: "No system audio was recorded" });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("System Audio Recording");
  await expectOneSmallLine(page, notice);
  const open = notice.getByRole("button", { name: "Open Settings" });
  await expect(open).toHaveCSS("background-color", TRANSPARENT);
  await open.click();
  expect(
    await callsTo(page, "open_system_audio_privacy_settings"),
  ).toHaveLength(1);
});

test("with the permission allowed, a silent system track is one quiet line", async ({
  page,
}) => {
  await openMeeting(
    page,
    /Point hebdo produit/,
    "&systemAudioSilent=1&systemAudioPermission=granted",
  );
  const notice = page.getByRole("status").filter({
    hasText: "No sound from other apps was captured.",
  });
  await expect(notice).toBeVisible();
  await expect(notice).not.toContainText("System Audio Recording");
  await expect(notice.getByRole("button")).toHaveCount(0);
  await expectOneSmallLine(page, notice);
});

test("the icon actions of each tab sit together as a toolbar", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);
  await expectToolbar(page, [
    "Search Summary",
    "Copy Summary",
    "Export as Markdown",
  ]);
  await page.getByRole("tab", { name: "Transcript" }).click();
  await expectToolbar(page, [
    "Search Transcript",
    "Copy Transcript",
    "Export as Markdown",
    "Rename Speakers",
  ]);
});

test("the audio row's actions are the same pink button, side by side", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/, "&audioBytes=350623");
  const again = page.getByRole("button", { name: "Transcribe Again" });
  const remove = page.getByRole("button", { name: "Delete Audio" });
  await expect(again).toBeVisible();
  await expect(remove).toBeVisible();
  const [againBox, removeBox] = await Promise.all([
    again.boundingBox(),
    remove.boundingBox(),
  ]);
  expect(againBox?.width).toBe(removeBox?.width);
  expect(againBox?.height).toBe(removeBox?.height);
  expect(againBox?.y).toBe(removeBox?.y);
  await expect(again).not.toHaveCSS("background-color", TRANSPARENT);
  for (const property of ["background-color", "color"]) {
    await expect(remove).toHaveCSS(property, await cssOf(again, property));
  }
  // The row ends where the tab content ends.
  const panel = await page.getByRole("tabpanel").boundingBox();
  expect((removeBox?.x ?? 0) + (removeBox?.width ?? 0)).toBeCloseTo(
    (panel?.x ?? 0) + (panel?.width ?? 0),
    0,
  );
});

test("deleting the meeting or its audio asks with a pink button, never red", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/, "&audioBytes=350623");
  const again = page.getByRole("button", { name: "Transcribe Again" });
  const pink = await cssOf(again, "background-color");
  await page.getByRole("button", { name: "Delete Audio" }).click();
  const confirm = page
    .getByRole("dialog")
    .getByRole("button", { name: "Delete Audio" });
  await expect(confirm).toHaveCSS("background-color", pink);
  const classes = await page.evaluate(() =>
    Array.from(document.querySelectorAll("button")).map((b) => b.className),
  );
  expect(classes.filter((name) => /\bred-/.test(name))).toEqual([]);
});

test("dark: follows Murmur's theme even when macOS's differs", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);
  // The live log viewer's error colours use these classes.
  const probe = await page.evaluateHandle(() => {
    const span = document.createElement("span");
    span.className = "text-red-600 dark:text-red-400";
    document.body.append(span);
    return span;
  });
  const colorUnder = async (
    appTheme: "light" | "dark" | "system",
    osScheme: "light" | "dark",
  ) => {
    await page.emulateMedia({ colorScheme: osScheme });
    await page.evaluate((theme) => {
      if (theme === "system") delete document.documentElement.dataset.theme;
      else document.documentElement.dataset.theme = theme;
    }, appTheme);
    return probe.evaluate((span) => getComputedStyle(span).color);
  };

  const light = await colorUnder("system", "light");
  const dark = await colorUnder("system", "dark");
  expect(light).not.toBe(dark);
  expect(await colorUnder("light", "dark")).toBe(light);
  expect(await colorUnder("dark", "light")).toBe(dark);
});

test("text buttons are filled from the start, icon buttons only tint on hover", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);
  const back = page.getByRole("button", { name: "All Meetings" });
  await expect(back).not.toHaveCSS("background-color", TRANSPARENT);

  const copy = page.getByRole("button", { name: "Copy Summary" });
  await expect(copy).toHaveCSS("background-color", TRANSPARENT);
  await copy.hover();
  await expect(copy).not.toHaveCSS("background-color", TRANSPARENT);
  await expect(copy).toHaveCSS("border-top-width", "0px");
});

test("audio under a megabyte shows its size in kilobytes", async ({ page }) => {
  await openMeeting(page, /Point hebdo produit/, "&audioBytes=350623");
  await expect(page.getByText("Audio kept · 351 kB")).toBeVisible();
});

test("a meeting shows its transcript in order, with a name at each change of speaker", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);

  await expect(page.getByLabel("Meeting Title")).toHaveValue(
    "Point hebdo produit",
  );
  // A ready meeting opens on its summary.
  await expect(page.getByRole("tab", { name: "Summary" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.getByRole("tab", { name: "Transcript" }).click();
  await expect(page.getByRole("tab", { name: "Transcript" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByText("Speakers: 2 · 1:12:05")).toBeVisible();

  const items = bubbles(page);
  await expect(items).toHaveCount(8);
  await expect(items.nth(0)).toContainText("Speaker 2");
  await expect(items.nth(0)).toContainText("on commence par le point produit");
  await expect(items.nth(1)).toContainText("Me");
  // The user's second bubble continues without repeating the name.
  await expect(items.nth(2).getByRole("button")).toHaveCount(0);
  await expect(items.nth(3)).toContainText("Speaker 2");
  await expect(page.locator("article [role=alert]")).toHaveCount(0);
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("the title saves on Enter and reverts on Escape", async ({ page }) => {
  await openMeeting(page, /Point hebdo produit/);
  const title = page.getByLabel("Meeting Title");

  await title.fill("Revue produit");
  await title.press("Escape");
  await expect(title).toHaveValue("Point hebdo produit");
  expect(await callsTo(page, "rename_meeting")).toEqual([]);

  await title.fill("Revue produit");
  await title.press("Enter");
  await expect
    .poll(() => callsTo(page, "rename_meeting"))
    .toEqual([{ id: 3, title: "Revue produit" }]);
  await expect(title).toHaveValue("Revue produit");
});

test("search ignores case and accents and cycles through the matches", async ({
  page,
}) => {
  await openTranscript(page);
  await page.getByRole("button", { name: "Search Transcript" }).click();
  const search = page.getByRole("textbox", { name: "Search Transcript" });

  await search.fill("INTEGRATION");
  await expect(page.getByText("1/1")).toBeVisible();
  await expect(page.locator("mark")).toHaveText("intégration");

  await search.fill("pr");
  await expect(page.getByText("1/4")).toBeVisible();
  await search.press("Enter");
  await expect(page.getByText("2/4")).toBeVisible();
  await search.press("Shift+Enter");
  await search.press("Shift+Enter");
  await expect(page.getByText("4/4")).toBeVisible();
  await expect(page.locator("mark[data-current]")).toHaveCount(1);

  await search.fill("zzz");
  await expect(page.getByText("No matches")).toBeVisible();
  await search.press("Escape");
  await expect(page.locator("mark")).toHaveCount(0);
  await expect(page.getByText("Speakers: 2 · 1:12:05")).toBeVisible();
});

test("clicking a speaker's name renames them everywhere", async ({ page }) => {
  await openTranscript(page);
  await bubbles(page).nth(0).getByRole("button", { name: "Speaker 2" }).click();

  const dialog = page.getByRole("dialog", { name: "Rename Speakers" });
  const field = dialog.getByRole("textbox", { name: "Name for Speaker 2" });
  await expect(field).toBeFocused();
  await field.fill("Camille");
  await dialog.getByRole("button", { name: "Save" }).click();

  await expect(dialog).toHaveCount(0);
  expect(await callsTo(page, "rename_speaker")).toEqual([
    { id: 3, key: "s1", name: "Camille" },
  ]);
  await expect(
    bubbles(page).getByRole("button", { name: "Camille" }),
  ).toHaveCount(4);
  await expect(
    bubbles(page).getByRole("button", { name: "Speaker 2" }),
  ).toHaveCount(0);
});

test("the rename dialog opens from the header and saves nothing unchanged", async ({
  page,
}) => {
  await openTranscript(page);
  await page.getByRole("button", { name: "Rename Speakers" }).click();
  const dialog = page.getByRole("dialog", { name: "Rename Speakers" });
  await expect(dialog.getByRole("textbox")).toHaveCount(2);
  await expect(dialog.getByRole("button", { name: "Save" })).toBeDisabled();
  await dialog.getByRole("textbox", { name: "Name for Me" }).fill(" ");
  await expect(dialog.getByRole("button", { name: "Save" })).toBeDisabled();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  expect(await callsTo(page, "rename_speaker")).toEqual([]);
});

test("copy puts the same Markdown as export on the clipboard", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await openTranscript(page);

  await page.getByRole("button", { name: "Copy Transcript" }).click();
  await expect(page.getByText("Transcript copied.")).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    "# Point hebdo produit\n\n## Transcript\n",
  );

  await page.getByRole("button", { name: "Export as Markdown" }).click();
  await expect(page.getByText("Transcript saved.")).toBeVisible();
  expect(await callsTo(page, "export_meeting_markdown")).toEqual([
    { id: 3, kind: "transcript" },
  ]);
});

test("a ready meeting's summary names people and shows its read time", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);

  await expect(page.getByText("1 min read")).toBeVisible();
  await expect(summary(page)).toContainText(
    "Weekly product check-in between Me and Speaker 2.",
  );
  await expect(
    summary(page).getByRole("heading", { name: "Payment integration" }),
  ).toBeVisible();
  await expect(summary(page).getByRole("listitem")).toHaveCount(5);
  await expect(summary(page)).not.toContainText("[[");
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("renaming a speaker renames them in the summary too", async ({ page }) => {
  await openTranscript(page);
  await bubbles(page).nth(0).getByRole("button", { name: "Speaker 2" }).click();
  const dialog = page.getByRole("dialog", { name: "Rename Speakers" });
  await dialog
    .getByRole("textbox", { name: "Name for Speaker 2" })
    .fill("Camille");
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog).toHaveCount(0);

  await page.getByRole("tab", { name: "Summary" }).click();
  await expect(summary(page)).toContainText("between Me and Camille.");
  await expect(summary(page)).toContainText("Camille — tell the support team");
  await expect(summary(page)).not.toContainText("Speaker 2");
});

test("summary search ignores case and cycles through the matches", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);
  await page.getByRole("button", { name: "Search Summary" }).click();
  const search = page.getByRole("textbox", { name: "Search Summary" });

  await search.fill("PAYMENT");
  await expect(page.getByText("1/1")).toBeVisible();
  await expect(page.locator("mark")).toHaveText("Payment");

  await search.fill("the");
  await expect(page.getByText("1/4")).toBeVisible();
  await search.press("Enter");
  await expect(page.getByText("2/4")).toBeVisible();
  await expect(page.locator("mark[data-current]")).toHaveCount(1);
  await search.press("Escape");
  await expect(page.locator("mark")).toHaveCount(0);
});

test("the search counter stays within the matches when the text changes", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/);
  await page.getByRole("button", { name: "Search Summary" }).click();
  const search = page.getByRole("textbox", { name: "Search Summary" });

  // "Speaker 2" three times, and "November".
  await search.fill("er");
  await expect(page.getByText("1/4")).toBeVisible();
  await search.press("Shift+Enter");
  await expect(page.getByText("4/4")).toBeVisible();

  await page.evaluate(async () => {
    await window.__TAURI_INTERNALS__.invoke("rename_speaker", {
      id: 3,
      key: "s1",
      name: "Camille",
    });
    await window.settingsPages.emitMeetingsUpdated(3);
  });
  await expect(summary(page)).toContainText("between Me and Camille.");
  await expect(page.getByText("1/1")).toBeVisible();
  await expect(page.locator("mark[data-current]")).toHaveText("er");
});

test("the summary copies and exports as Markdown", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await openMeeting(page, /Point hebdo produit/);

  await page.getByRole("button", { name: "Copy Summary" }).click();
  await expect(page.getByText("Summary copied.")).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    "# Point hebdo produit\n\n## Summary\n",
  );
  await page.getByRole("button", { name: "Export as Markdown" }).click();
  await expect(page.getByText("Summary saved.")).toBeVisible();
  expect(await callsTo(page, "export_meeting_markdown")).toEqual([
    { id: 3, kind: "summary" },
  ]);
});

test("a failed summary shows why, and Retry writes it", async ({ page }) => {
  await openMeeting(page, /Point hebdo produit/, "&summary=failed");
  const alert = page.locator("article [role=alert]");
  await expect(alert).toContainText(
    "Codex is limiting requests right now. Retry later.",
  );
  // The transcript is unaffected.
  await expect(
    page.getByRole("button", { name: "Copy Summary" }),
  ).toBeDisabled();

  await alert.getByRole("button", { name: "Retry" }).click();
  await expect
    .poll(() => callsTo(page, "retry_meeting_summary"))
    .toEqual([{ id: 3 }]);
  await expect(summary(page)).toContainText("Weekly product check-in");
  await expect(alert).toHaveCount(0);
});

test("a summary on its way says so", async ({ page }) => {
  await openMeeting(page, /Point hebdo produit/, "&summary=pending");
  await expect(page.locator("article [role=status]")).toContainText(
    "Writing the summary…",
  );
  await openMeeting(page, /09:41 · Interrupted/);
  await page.getByRole("tab", { name: "Summary" }).click();
  await expect(
    page.getByText(
      "The summary is written once the whole meeting is transcribed.",
    ),
  ).toBeVisible();
});

test("deleting the audio asks first and keeps the text", async ({ page }) => {
  await openMeeting(page, /Point hebdo produit/);
  await expect(page.getByText("Audio kept · 35 MB")).toBeVisible();

  await page.getByRole("button", { name: "Delete Audio" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Delete this meeting's audio?",
  });
  await dialog.getByRole("button", { name: "Delete Audio" }).click();
  await expect
    .poll(() => callsTo(page, "delete_meeting_audio"))
    .toEqual([{ id: 3 }]);
  await expect(
    page.getByText("Audio deleted. The transcript and summary stay."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Transcribe Again" }),
  ).toHaveCount(0);
  await expect(summary(page)).toContainText("Weekly product check-in");
});

test("transcribing again asks first", async ({ page }) => {
  await openMeeting(page, /Point hebdo produit/);
  await page.getByRole("button", { name: "Transcribe Again" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Transcribe this meeting again?",
  });
  await dialog.getByRole("button", { name: "Cancel" }).click();
  expect(await callsTo(page, "retranscribe_meeting")).toEqual([]);

  await page.getByRole("button", { name: "Transcribe Again" }).click();
  await dialog.getByRole("button", { name: "Transcribe Again" }).click();
  await expect
    .poll(() => callsTo(page, "retranscribe_meeting"))
    .toEqual([{ id: 3 }]);
});

test("processing shows live progress, including a rate-limit pause", async ({
  page,
}) => {
  await openMeeting(page, /47:12 · Processing/);
  const status = page.locator("article [role=status]");
  // Before the next live update, the stage saved with the meeting.
  await expect(status).toContainText("Transcribing…");
  await expect(
    page.getByText(
      "The transcript appears here once the meeting is processed.",
    ),
  ).toBeVisible();

  const emit = (progress: object) =>
    page.evaluate(
      (payload) =>
        window.settingsPages.emitProgress(
          payload as Parameters<typeof window.settingsPages.emitProgress>[0],
        ),
      progress,
    );
  await emit({
    chunks_done: 2,
    chunks_total: 5,
    id: 4,
    rate_limited: false,
    stage: "transcribing",
  });
  await expect(status).toContainText("Transcribing: 2 of 5 parts done…");
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "40",
  );
  await emit({
    chunks_done: 2,
    chunks_total: 5,
    id: 4,
    rate_limited: true,
    stage: "transcribing",
  });
  await expect(status).toContainText("Codex asked to slow down.");
  // Another meeting's progress is ignored.
  await emit({
    chunks_done: 1,
    chunks_total: 1,
    id: 3,
    rate_limited: false,
    stage: "aligning",
  });
  await expect(status).toContainText("Codex asked to slow down.");
  await emit({
    chunks_done: 5,
    chunks_total: 5,
    id: 4,
    rate_limited: false,
    stage: "aligning",
  });
  await expect(status).toContainText("Putting the conversation in order…");
});

test("a meeting opened while speakers are told apart says so, not that it waits", async ({
  page,
}) => {
  await openMeeting(page, /47:12 · Processing/, "&stage=analyzing");
  const status = page.locator("article [role=status]");
  await expect(status).toContainText("Telling speakers apart…");
  await expect(status).not.toContainText("Waiting to be processed");
});

test("a meeting held by a rate limit says when it resumes and can go now", async ({
  page,
}) => {
  await openMeeting(page, /47:12 · Processing/, "&held=1");
  const status = page.locator("article [role=status]");
  await expect(status).toContainText(
    /Codex is limiting requests\. Processing resumes at \d{1,2}:\d{2}/,
  );
  await status.getByRole("button", { name: "Process Now" }).click();
  await expect.poll(() => callsTo(page, "retry_meeting")).toEqual([{ id: 4 }]);
});

test("a ready meeting whose summary is held says when it resumes, not that it writes", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/, "&summary=pending&held=1");
  const status = page.locator("article [role=status]");
  await expect(status).toContainText(
    /Codex is limiting requests\. Processing resumes at \d{1,2}:\d{2}/,
  );
  await expect(page.getByText("Writing the summary…")).toHaveCount(0);
  await status.getByRole("button", { name: "Process Now" }).click();
  await expect.poll(() => callsTo(page, "retry_meeting")).toEqual([{ id: 3 }]);
});

test("a speaker name being typed survives the meeting reloading", async ({
  page,
}) => {
  await openTranscript(page);
  await page.getByRole("button", { name: "Rename Speakers" }).click();
  const dialog = page.getByRole("dialog", { name: "Rename Speakers" });
  // The dialog focuses the first field a frame after opening; typing before
  // that would land the name there.
  await expect(
    dialog.getByRole("textbox", { name: "Name for Me" }),
  ).toBeFocused();
  const field = dialog.getByRole("textbox", { name: "Name for Speaker 2" });
  await field.fill("Cam");
  const loads = (await callsTo(page, "get_meeting")).length;
  // The summary or the compression finishing reloads the meeting.
  await page.evaluate(() => window.settingsPages.emitMeetingsUpdated(3));
  await expect
    .poll(async () => (await callsTo(page, "get_meeting")).length)
    .toBe(loads + 1);
  await page.waitForTimeout(100);
  await expect(field).toHaveValue("Cam");
});

test("a meeting with failed parts shows them in place and retries them", async ({
  page,
}) => {
  await openMeeting(page, /Point hebdo produit/, "&meetingFailed=1");

  await expect(page.locator("article [role=alert]")).toContainText(
    "Some parts couldn't be transcribed.",
  );
  const missing = bubbles(page).filter({
    hasText: "Part 10:00–20:00 couldn't be transcribed.",
  });
  await expect(missing).toHaveCount(1);
  await missing.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => callsTo(page, "retry_meeting")).toEqual([{ id: 3 }]);
  // After the gap, the remote speaker is named again.
  await expect(bubbles(page).nth(6)).toContainText("Speaker 2");
});

test("an interrupted recording can be transcribed, and Back returns to the list", async ({
  page,
}) => {
  await openMeeting(page, /09:41 · Interrupted/);
  await expect(page.locator("article [role=alert]")).toContainText(
    "The recording was interrupted.",
  );
  await page.getByRole("button", { name: "Transcribe" }).click();
  await expect.poll(() => callsTo(page, "retry_meeting")).toEqual([{ id: 2 }]);

  await page.getByRole("button", { name: "All Meetings" }).click();
  await expect(
    page.getByRole("button", { name: "Start Meeting" }),
  ).toBeVisible();
  await expect(page.getByRole("listitem")).toHaveCount(3);
});
