/**
 * "Me" takes the accent's text colour; the others cycle through distinct
 * hues. Each hue is mixed with the theme's text colour, so it darkens on the
 * light theme and lightens on the dark one, whichever theme the user forces;
 * at 60% the palest, amber, still reads at 4.5:1 on the light background.
 */
const HUES = [
  "#10b981",
  "#8b5cf6",
  "#0ea5e9",
  "#f59e0b",
  "#f43f5e",
  "#14b8a6",
  "#6366f1",
];

export const speakerColor = (index: number): string =>
  index === 0
    ? "var(--color-accent-text)"
    : `color-mix(in srgb, ${HUES[(index - 1) % HUES.length]} 60%, var(--color-text))`;
