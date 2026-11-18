import { expect, test, type Page } from "@playwright/test";
import { withoutWindowsSection } from "../src/components/whats-new/platformNotes";

const NOTE = [
  "# Murmur 1.16.1",
  "",
  "Overlay",
  "",
  "- An error message no longer has a red outline around it.",
  "",
  "Windows (beta)",
  "",
  "Murmur for Windows is a beta: download `Murmur_1.16.1_x64-setup.exe` from the same page as the Mac app.",
  "",
].join("\n");

test("the Windows app leaves out where to download the Windows installer", () => {
  const shown = withoutWindowsSection(NOTE);
  expect(shown).toContain("An error message no longer has a red outline");
  expect(shown).not.toContain("Windows (beta)");
  expect(shown).not.toContain("setup.exe");

  // The section written as a heading.
  expect(
    withoutWindowsSection(NOTE.replace("Windows (beta)", "## Windows (beta)")),
  ).toBe(shown);

  // A checkout with Windows line endings.
  const crlf = withoutWindowsSection(NOTE.split("\n").join("\r\n"));
  expect(crlf).toContain("An error message no longer has a red outline");
  expect(crlf).not.toContain("Windows (beta)");
});

test("a note with nothing else, or without the section, is shown whole", () => {
  const windowsOnly = [
    "# Murmur 1.16.2",
    "",
    "Windows (beta)",
    "",
    "The installer no longer asks twice.",
    "",
  ].join("\n");
  expect(withoutWindowsSection(windowsOnly)).toBe(windowsOnly);

  const plain = "# Murmur 1.12.1\n\n- A fix.\n";
  expect(withoutWindowsSection(plain)).toBe(plain);

  // The words inside a sentence are not the section.
  const inline = "# Murmur 2.0.0\n\n- Windows (beta) gets Meetings.\n";
  expect(withoutWindowsSection(inline)).toBe(inline);
});

const updated = (platform: "windows" | "macos") =>
  `/tests/fixtures/settings-pages.html?${
    platform === "windows" ? "platform=windows&" : ""
  }appVersion=1.16.1&seenNotes=1.15.0`;

const lastSeenWrites = (page: Page) =>
  page.evaluate(() =>
    window.settingsPages.calls
      .filter(
        (call) => call.command === "change_whats_new_last_seen_version_setting",
      )
      .map((call) => call.args),
  );

test("a PC shows what is new after an update, once", async ({ page }) => {
  await page.goto(updated("windows"));

  const notes = page.getByRole("dialog", { name: "New in Murmur v1.16.1" });
  await expect(notes).toBeVisible();
  await expect(notes).toContainText(
    "An error message no longer has a red outline",
  );
  // The PC just ran the installer: it is not told where to download it.
  await expect(notes).not.toContainText("Windows (beta)");
  await expect(notes).not.toContainText("setup.exe");

  await notes.getByRole("button", { name: "Close" }).click();
  await expect(notes).toHaveCount(0);
  expect(await lastSeenWrites(page)).toEqual([{ version: "1.16.1" }]);
});

test("a Mac still reads where the Windows build is", async ({ page }) => {
  await page.goto(updated("macos"));

  const notes = page.getByRole("dialog", { name: "New in Murmur v1.16.1" });
  await expect(notes).toContainText("Windows (beta)");
  await expect(notes).toContainText("Murmur_1.16.1_x64-setup.exe");
});
