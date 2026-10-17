export const BAR_COUNT = 13;
export const BAR_MIN_PX = 4;
// The bars are laid out at this height (.ov-wave in RecordingOverlay.css) and
// drawn shorter with scaleY, so a voice frame never changes layout.
export const BAR_MAX_PX = 18;

// The capsule grows in whole 2px steps, 1px per edge, so both edges stay on
// whole pixels (--ov-grow-step in RecordingOverlay.css).
export const GROW_STEPS = 9;

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

// Frame-rate independent easing towards `target`. The defaults suit the bars:
// quick to rise so syllables land on time, slower to fall so they do not
// flicker between frames.
export function approach(
  current: number,
  target: number,
  dtSeconds: number,
  riseSeconds = 0.06,
  fallSeconds = 0.18,
): number {
  const tau = target > current ? riseSeconds : fallSeconds;
  return target + (current - target) * Math.exp(-dtSeconds / tau);
}

export type Envelope = { level: number; quietSeconds: number };

// The capsule's envelope works like a VU meter's peak hold: it rises over a
// third of a second, holds through the short pauses between words, and only
// then releases, so the capsule follows phrases rather than syllables.
export function followPhrase(
  envelope: Envelope,
  voice: number,
  dtSeconds: number,
): Envelope {
  if (voice >= envelope.level) {
    return {
      level: approach(envelope.level, voice, dtSeconds, 0.35),
      quietSeconds: 0,
    };
  }
  const quietSeconds = envelope.quietSeconds + dtSeconds;
  if (quietSeconds < 0.5) return { level: envelope.level, quietSeconds };
  return {
    level: approach(envelope.level, voice, dtSeconds, 0.35, 0.7),
    quietSeconds,
  };
}

// Steps change only a quarter step past each boundary, so an envelope that
// rests on a boundary cannot flip the width back and forth.
export function growStep(current: number, level: number): number {
  const target = level * GROW_STEPS;
  if (target > current + 0.75) return current + 1;
  if (target < current - 0.75) return current - 1;
  return current;
}

// Shown instead of live levels when the user asks for reduced motion.
export const STATIC_HEIGHTS = barHeights(Array(16).fill(0.3));
