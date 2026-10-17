/**
 * "Me" takes the accent; the others cycle through distinct hues. Each hue
 * is mixed with the theme's text colour, so it darkens on the light theme
 * and lightens on the dark one, whichever theme the user forces.
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
    ? "var(--color-logo-primary)"
    : `color-mix(in srgb, ${HUES[(index - 1) % HUES.length]} 72%, var(--color-text))`;
