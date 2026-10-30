import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/fixtures/settings-race-contract.html");
  await page.waitForFunction(() => Boolean(window.settingsRaceContract));
});

const cases = [
  [
    "shared-initialization",
    { theme: "system", reads: 1, listens: 1, shared: true },
  ],
  ["retry-failures", { theme: "system", reads: 1, listens: 3 }],
  ["initial-event", { theme: "dark", reads: 2, listens: 1 }],
  ["retry-snapshot", { theme: "dark", reads: 2, listens: 2 }],
  ["disposed-events", { events: 0, unlistens: 1 }],
  [
    "binding-before-refresh",
    { keys: "Ctrl+Shift+X", readsBeforeRelease: 0, reads: 1 },
  ],
  ["read-before-binding", { keys: "Ctrl+Shift+X", reads: 2 }],
  [
    "same-binding-failure",
    {
      callsBeforeRelease: 1,
      updatingBeforeRefusal: true,
      secondAccepted: false,
      rolledBackKeys: "Ctrl+Shift+X",
      keys: "Ctrl+Shift+Z",
      historyLimit: 200,
      updating: false,
    },
  ],
  [
    "same-binding-first-refusal",
    {
      callsBeforeRelease: 1,
      firstAccepted: false,
      pendingKeys: "ctrl+shift+y",
      updatingBeforeAcceptance: true,
      keys: "Ctrl+Shift+Y",
    },
  ],
  [
    "binding-ipc-refusal",
    {
      firstAccepted: false,
      rolledBackKeys: "ctrl+space",
      keys: "Ctrl+Shift+Y",
      updating: false,
    },
  ],
  [
    "reset-after-update",
    {
      callsBeforeRelease: 1,
      readsBeforeRelease: 0,
      commands: "change_binding,reset_binding",
      keys: "Ctrl+Space",
    },
  ],
  [
    "independent-rollback",
    {
      callsBeforeRelease: 1,
      firstAccepted: false,
      keys: "ctrl+space",
      otherKeys: "F8",
    },
  ],
  [
    "reset-refusal",
    { keys: "ctrl+space", readsBeforeRelease: 0, updating: false },
  ],
] as const;

for (const [scenario, expected] of cases) {
  test(`preserves the settings contract: ${scenario}`, async ({ page }) => {
    await expect(
      page.evaluate((name) => window.settingsRaceContract.run(name), scenario),
    ).resolves.toEqual(expected);
  });
}
