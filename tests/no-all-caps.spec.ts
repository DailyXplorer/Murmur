import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const fixture = "/tests/fixtures/settings-pages.html";

/** Real acronyms, the only words allowed in full capitals. */
const ACRONYMS = [
  "AI",
  "BUT", // Brno University of Technology, in the model licences
  "BY", // the CC BY licence
  "CC",
  "GB",
  "GGUF", // the file format of the Intel model
  "MB",
  "NVIDIA",
  "TDT", // Token-and-Duration Transducer, in the Parakeet model name
];

/** Proper nouns and named macOS or app surfaces a sentence may capitalize. */
const PROPER_PHRASES = [
  "Screen & System Audio Recording",
  "Privacy & Security → Microphone",
  "System Audio Recording",
  "Privacy & Security",
  "System Settings",
  "Apache License",
  "Fluid Inference",
  "Hugging Face",
  "Community-1",
  "Speech@FIT",
  "Bluetooth",
  "Intel Macs",
  "European",
  "Parakeet",
  "Thursday",
  "ChatGPT",
  "English",
  "French",
  "WeSpeaker",
  "Markdown",
  "Core ML",
  "Log level",
  "Accessibility",
  "Meetings",
  "History",
  "Learned",
  "Minimal",
  "GitHub",
  "Murmur",
  "Apple",
  "Codex",
  "A–Z",
  "Mac",
];

const PAGES: { name: string; open: (page: Page) => Promise<void> }[] = [
  ...[
    { label: "General" },
    { label: "Transcription" },
    { label: "History" },
    { label: "Meetings", query: "?meetings=1" },
    { label: "Insights" },
    { label: "Advanced" },
    { label: "Debug" },
    { label: "About" },
  ].map(({ label, query = "" }) => ({
    name: label,
    open: async (page: Page) => {
      await page.goto(`${fixture}${query}`);
      await page
        .getByRole("navigation")
        .getByRole("button", { name: label, exact: true })
        .click();
    },
  })),
  {
    name: "Meeting transcript",
    open: async (page) => {
      await page.goto(`${fixture}?meetings=1`);
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.getByRole("button", { name: /Point hebdo produit/ }).click();
      await page.getByRole("tab", { name: "Transcript" }).click();
    },
  },
  {
    name: "Onboarding",
    open: async (page) => {
      await page.goto(`${fixture}?onboarding=1`);
    },
  },
];

const capitalsOnScreen = (page: Page, acronyms: string[]) =>
  page.evaluate((allowed) => {
    const found: string[] = [];
    const visible = (element: Element) =>
      element.checkVisibility({
        opacityProperty: true,
        visibilityProperty: true,
      });
    for (const element of document.body.querySelectorAll("*")) {
      if (!visible(element)) continue;
      const style = getComputedStyle(element);
      const ownText = Array.from(element.childNodes)
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent ?? "")
        .join("")
        .trim();
      if (!ownText) continue;
      if (style.textTransform === "uppercase") {
        found.push(`text-transform uppercase: "${ownText}"`);
      }
      if (style.fontVariantCaps.includes("small-caps")) {
        found.push(`${style.fontVariantCaps}: "${ownText}"`);
      }
    }
    const walker = document.createTreeWalker(
      document.body,
      NodeFilter.SHOW_TEXT,
    );
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!parent || !visible(parent)) continue;
      const text = node.textContent ?? "";
      for (const word of text.match(/[\p{L}\p{N}]+/gu) ?? []) {
        const letters = word.replace(/\P{L}/gu, "");
        if (
          letters.length >= 3 &&
          letters === letters.toUpperCase() &&
          letters !== letters.toLowerCase() &&
          !allowed.includes(word)
        ) {
          found.push(`"${word}" in "${text.trim()}"`);
        }
      }
    }
    return found;
  }, acronyms);

test("no visible text is set in capitals on any English page", async ({
  page,
}) => {
  const offenders: string[] = [];
  for (const { name, open } of PAGES) {
    await open(page);
    // Pages fill in from the mocked backend after the first paint.
    await page.waitForTimeout(500);
    for (const found of await capitalsOnScreen(page, ACRONYMS)) {
      offenders.push(`${name}: ${found}`);
    }
  }
  expect(offenders, offenders.join("\n")).toEqual([]);
});

const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const PROPER_PATTERN = new RegExp(
  `(?<![\\p{L}\\p{N}])(?:${[...PROPER_PHRASES]
    // Longest first, so a phrase wins over a shorter one inside it.
    .sort((a, b) => b.length - a.length)
    .map(escape)
    .join("|")})(?![\\p{L}\\p{N}])`,
  "gu",
);

const capitalizedMidSentence = (value: string) => {
  const text = value
    .replace(/\{\{[^}]*\}\}/g, "x")
    .replace(/<[^>]*>/g, "")
    .replace(PROPER_PATTERN, "x");
  const words = [];
  for (const match of text.matchAll(/[\p{L}\p{N}][\p{L}\p{N}'’-]*/gu)) {
    const word = match[0];
    if (!/^\p{Lu}/u.test(word) || /^I(['’]|$)/u.test(word)) continue;
    const letters = word.replace(/\P{L}/gu, "");
    if (letters.length >= 2 && letters === letters.toUpperCase()) {
      if (!ACRONYMS.includes(word)) words.push(word);
      continue;
    }
    const line = text.slice(0, match.index).split("\n").pop() ?? "";
    // A word after "+" is a key in a shortcut such as "Cmd+Enter".
    if (line.endsWith("+")) continue;
    const lead = line.replace(/[\s"“‘'(«[\-•*]+$/u, "");
    if (lead !== "" && !/[.!?:…]$/u.test(lead)) words.push(word);
  }
  return words;
};

type Tree = { [key: string]: string | Tree };
const entries = (tree: Tree, prefix = ""): [string, string][] =>
  Object.entries(tree).flatMap(([key, value]) =>
    typeof value === "string"
      ? [[`${prefix}${key}`, value]]
      : entries(value, `${prefix}${key}.`),
  );

test("English strings capitalize only proper nouns and sentence starts", () => {
  const english = JSON.parse(
    readFileSync(
      fileURLToPath(
        new URL("../src/i18n/locales/en/translation.json", import.meta.url),
      ),
      "utf8",
    ),
  ) as Tree;
  const offenders = entries(english).flatMap(([key, value]) => {
    const words = capitalizedMidSentence(value);
    return words.length ? [`${key}: ${words.join(", ")} in "${value}"`] : [];
  });
  expect(offenders, offenders.join("\n")).toEqual([]);
});
