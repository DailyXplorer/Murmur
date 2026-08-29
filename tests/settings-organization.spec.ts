import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

const readSource = (path: string) =>
  readFile(`${repositoryRoot}${path}`, "utf8");

test.describe("settings information architecture", () => {
  test("About contains project information instead of preferences", async () => {
    const source = await readSource(
      "src/components/settings/about/AboutSettings.tsx",
    );

    expect(source).toContain("settings.about.version.title");
    expect(source).toContain("settings.about.supportDevelopment.title");
    expect(source).toContain("settings.about.sourceCode.title");
    expect(source).not.toMatch(
      /AppLanguageSelector|ThemeSelector|AccentColorSelector|ShowWhatsNewOnUpdate|AppDataDirectory|LogDirectory/,
    );
  });

  test("each preference is composed by the page that owns it", async () => {
    const [general, transcription, history, advanced] = await Promise.all([
      readSource("src/components/settings/general/GeneralSettings.tsx"),
      readSource(
        "src/components/settings/transcription/TranscriptionSettings.tsx",
      ),
      readSource("src/components/settings/history/HistorySettings.tsx"),
      readSource("src/components/settings/advanced/AdvancedSettings.tsx"),
    ]);

    expect(general).toMatch(
      /<AppLanguageSelector[\s\S]*<ThemeSelector[\s\S]*<AccentColorSelector/,
    );
    expect(transcription).toMatch(
      /<FillerWordRemoval[\s\S]*<CustomWords[\s\S]*<AppendTrailingSpace/,
    );
    expect(transcription).toMatch(
      /<PasteMethodSetting[\s\S]*<ClipboardHandlingSetting[\s\S]*<AutoSubmit/,
    );
    expect(history).toMatch(
      /<HistoryLimit[\s\S]*<RecordingRetentionPeriodSelector/,
    );
    expect(advanced).toMatch(/<UpdateChecksToggle[\s\S]*<ShowWhatsNewOnUpdate/);
    expect(advanced).toMatch(/<AppDataDirectory[\s\S]*<LogDirectory/);
  });

  test("sidebar order follows the main workflow and uses buttons", async () => {
    const source = await readSource("src/components/Sidebar.tsx");
    const general = source.indexOf('"general"');
    const transcription = source.indexOf('"transcription"', general);
    const history = source.indexOf('"history"', transcription);
    const advanced = source.indexOf('"advanced"', history);
    const about = source.indexOf('"about"', advanced);

    expect(general).toBeGreaterThan(-1);
    expect(transcription).toBeGreaterThan(general);
    expect(history).toBeGreaterThan(transcription);
    expect(advanced).toBeGreaterThan(history);
    expect(about).toBeGreaterThan(advanced);
    expect(source).toContain('aria-current={isActive ? "page" : undefined}');
    expect(source).not.toMatch(/<div[^>]*onClick=/);
  });
});
