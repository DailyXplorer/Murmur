// The dictation orb: a ring in a 34-point circle (--ov-orb in
// RecordingOverlay.css) whose outline wobbles with the voice.
export const ORB_SIZE = 34;
export const ORB_RADIUS = 8.5;
// How far the loudest voice pushes the outline past the ring, in points.
const VOICE_WOBBLE = 4.2;
// The resting breath, in points: alive while the microphone is open and
// nothing is said.
const REST_BREATH = 0.4;
const OUTLINE_POINTS = 96;

// Overall voice energy in 0..1, from the bands that carry speech.
export function voiceLevel(levels: readonly number[]): number {
  const speech = levels.slice(1, 5);
  if (speech.length === 0) return 0;
  return Math.min(1, speech.reduce((sum, v) => sum + v, 0) / speech.length);
}

// Frame-rate independent easing towards `target`: quick to rise so syllables
// land on time, slower to fall so the outline does not flicker.
export function approach(
  current: number,
  target: number,
  dtSeconds: number,
  riseSeconds = 0.06,
  fallSeconds = 0.2,
): number {
  const tau = target > current ? riseSeconds : fallSeconds;
  return target + (current - target) * Math.exp(-dtSeconds / tau);
}

// The outline as an SVG path, at `level` (0..1) and `seconds` of animation
// time: a circle that breathes at rest and ripples with the voice.
export function orbPath(level: number, seconds: number): string {
  const centre = ORB_SIZE / 2;
  const points: string[] = [];
  for (let k = 0; k < OUTLINE_POINTS; k++) {
    const angle = (k / OUTLINE_POINTS) * Math.PI * 2;
    const ripple =
      0.6 * Math.sin(angle * 3 + seconds * 6) +
      0.4 * Math.sin(angle * 5 - seconds * 9);
    const radius =
      ORB_RADIUS +
      REST_BREATH * (1 - level) * Math.sin(seconds * 2) +
      VOICE_WOBBLE * level * ripple;
    const x = centre + radius * Math.cos(angle);
    const y = centre + radius * Math.sin(angle);
    points.push(`${k ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`);
  }
  return `${points.join("")}Z`;
}

// The still outline: before the microphone delivers, and with reduced motion.
export const RESTING_PATH = orbPath(0, 0);
