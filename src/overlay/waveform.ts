export const BAR_COUNT = 13;
export const BAR_MIN_PX = 4;
export const BAR_MAX_PX = 22;

const CENTER = (BAR_COUNT - 1) / 2;

// The host sends 16 log-spaced spectrum bands in 0..1. Band 0 is mostly room
// rumble, so the centre bar starts at band 1 (~100-375 Hz, the voice
// fundamental) and each step outwards reads the next band up. Mirroring keeps
// the shape symmetric, and the taper keeps the edges lower than the middle.
export function barHeights(levels: readonly number[]): number[] {
  return Array.from({ length: BAR_COUNT }, (_, i) => {
    const distance = Math.abs(i - CENTER);
    const level = Math.min(1, Math.max(0, levels[distance + 1] ?? 0));
    const taper = 1 - 0.45 * (distance / CENTER) ** 2;
    return BAR_MIN_PX + (BAR_MAX_PX - BAR_MIN_PX) * level ** 0.65 * taper;
  });
}

// Overall voice energy in 0..1, from the bands that carry speech.
export function voiceLevel(levels: readonly number[]): number {
  const speech = levels.slice(1, 5);
  if (speech.length === 0) return 0;
  return Math.min(1, speech.reduce((sum, v) => sum + v, 0) / speech.length);
}

// Frame-rate independent easing towards `target`: quick to rise so syllables
// land on time, slower to fall so the bars do not flicker between frames.
export function approach(
  current: number,
  target: number,
  dtSeconds: number,
): number {
  const tau = target > current ? 0.06 : 0.18;
  return target + (current - target) * Math.exp(-dtSeconds / tau);
}

// Shown instead of live levels when the user asks for reduced motion.
export const STATIC_HEIGHTS = barHeights(Array(16).fill(0.3));
