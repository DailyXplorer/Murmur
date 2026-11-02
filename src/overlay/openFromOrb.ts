import { useSyncExternalStore } from "react";

const reducedMotionQuery = window.matchMedia(
  "(prefers-reduced-motion: reduce)",
);
const subscribeReducedMotion = (onChange: () => void) => {
  reducedMotionQuery.addEventListener("change", onChange);
  return () => reducedMotionQuery.removeEventListener("change", onChange);
};
const getReducedMotion = () => reducedMotionQuery.matches;

/** Whether the user asks for reduced motion, following any change. */
export const useReducedMotion = () =>
  useSyncExternalStore(subscribeReducedMotion, getReducedMotion);

// Grows a pill from the orb's circle (--ov-orb in RecordingOverlay.css) to
// its own size. CSS cannot transition to an auto size, so the size is
// measured before the first paint and the growth runs towards it. The
// pill's body keeps its size and is clipped meanwhile, so its text never
// rewraps as the pill grows.
export function openFromOrb(pill: HTMLElement): Animation {
  const orb = getComputedStyle(pill).getPropertyValue("--ov-orb");
  return pill.animate(
    [
      { width: orb, height: orb },
      { width: `${pill.offsetWidth}px`, height: `${pill.offsetHeight}px` },
    ],
    { duration: 420, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
  );
}
