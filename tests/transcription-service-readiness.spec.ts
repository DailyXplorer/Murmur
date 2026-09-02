import { expect, test } from "@playwright/test";

test("accepts a ready Meta AI app as the only transcription service", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/module-test.html");

  const ready = await page.evaluate(async () => {
    const { hasUsableTranscriptionService } =
      await import("/src/lib/transcriptionServiceReadiness.ts");
    return hasUsableTranscriptionService({
      codex: { signed_in: false },
      gemini: null,
      meta: null,
      metaApp: { ready: true },
    });
  });

  expect(ready).toBe(true);
});
