import { expect, test, type Page } from "@playwright/test";

const open = async (page: Page, query: string) => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
};

/** The setting row titled ChatGPT. */
const row = (page: Page) =>
  page
    .getByRole("heading", { name: "ChatGPT", exact: true })
    .locator("xpath=ancestor::div[contains(@class,'px-4')][1]");

const callsTo = (page: Page, command: string) =>
  page.evaluate(
    (name) =>
      window.settingsPages.calls.filter((call) => call.command === name).length,
    command,
  );

const expectNoUnhandledCommands = async (page: Page) =>
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);

for (const engine of ["cloud", "local"]) {
  test(`one ChatGPT row shows the session and never the e-mail (${engine})`, async ({
    page,
  }) => {
    await open(page, `engine=${engine}`);
    await expect(row(page)).toHaveCount(1);
    await expect(
      row(page).getByText("Signed in", { exact: true }),
    ).toBeVisible();
    await expect(row(page).getByRole("button")).toHaveText(["Sign out"]);
    await expect(row(page)).not.toContainText("@");
    await expect(page.getByRole("heading", { name: "Codex" })).toHaveCount(0);
    await expect(page.getByText("Open Codex setup")).toHaveCount(0);
    await expectNoUnhandledCommands(page);
  });
}

test("Sign out asks first, says Codex is signed out too, and Cancel keeps the session", async ({
  page,
}) => {
  await open(page, "engine=cloud");
  await row(page).getByRole("button", { name: "Sign out" }).click();
  const dialog = page.getByRole("dialog", { name: "Sign out of ChatGPT?" });
  await expect(
    dialog.getByText(
      "Murmur and the Codex app share one ChatGPT session on this Mac, so signing out here also signs out Codex.",
    ),
  ).toBeVisible();

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(row(page).getByRole("button")).toHaveText(["Sign out"]);
  expect(await callsTo(page, "sign_out_of_chatgpt")).toBe(0);

  await row(page).getByRole("button", { name: "Sign out" }).click();
  await dialog.getByRole("button", { name: "Sign out" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(row(page).getByRole("button")).toHaveText(["Sign in"]);
  await expect(
    row(page).getByText("Not signed in", { exact: true }),
  ).toBeVisible();
  expect(await callsTo(page, "sign_out_of_chatgpt")).toBe(1);
  await expectNoUnhandledCommands(page);
});

test("a failed sign-out says so and keeps the session", async ({ page }) => {
  await open(page, "engine=cloud&signOut=fails");
  await row(page).getByRole("button", { name: "Sign out" }).click();
  await page
    .getByRole("dialog", { name: "Sign out of ChatGPT?" })
    .getByRole("button", { name: "Sign out" })
    .click();
  await expect(page.getByText("Couldn't sign out of ChatGPT.")).toBeVisible();
  await expect(page.getByText("Couldn't remove auth.json")).toHaveCount(0);
  await expect(row(page).getByRole("button")).toHaveText(["Sign out"]);
});

test("Sign in waits for the browser, then shows the session", async ({
  page,
}) => {
  await open(page, "engine=cloud&account=none");
  await expect(
    row(page).getByText("Not signed in", { exact: true }),
  ).toBeVisible();
  await row(page).getByRole("button", { name: "Sign in" }).click();
  await expect(
    row(page).getByText("Finish signing in in your browser.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(row(page).getByRole("button")).toHaveText(["Cancel"]);
  expect(await callsTo(page, "sign_in_with_chatgpt")).toBe(1);

  await page.evaluate(() => window.settingsPages.resolveSignIn());
  await expect(row(page).getByRole("button")).toHaveText(["Sign out"]);
  await expect(row(page).getByText("Signed in", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("checkbox", { name: "Cleanup and formatting" }),
  ).toBeEnabled();
  await expectNoUnhandledCommands(page);
});

test("Cancel stops a sign-in from settings", async ({ page }) => {
  await open(page, "engine=local&account=none");
  await expect(
    row(page).getByText("Optional: connect for AI cleanup and more languages."),
  ).toBeVisible();
  await row(page).getByRole("button", { name: "Sign in" }).click();
  await row(page).getByRole("button", { name: "Cancel" }).click();
  await expect(row(page).getByRole("button")).toHaveText(["Sign in"]);
  expect(await callsTo(page, "cancel_chatgpt_sign_in")).toBe(1);
  await expectNoUnhandledCommands(page);
});

test("an expired session asks to sign in again", async ({ page }) => {
  await open(page, "engine=cloud&account=expired");
  await expect(
    row(page).getByText("Session expired, sign in again", { exact: true }),
  ).toBeVisible();
  await expect(row(page).getByRole("button")).toHaveText(["Sign in"]);
});
