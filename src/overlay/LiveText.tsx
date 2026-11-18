import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

/** More than the widest pill shows in any script. */
const MAX_CHARS = 400;

/** The end of `text`, cut between code points. */
const lastChars = (text: string) =>
  text.length <= MAX_CHARS ? text : Array.from(text).slice(-MAX_CHARS).join("");

/** The words of a dictation on one line beside its orb. The line is laid
 * out from its end: once the pill is at its widest, the newest words stay
 * and the oldest leave by the start, in either writing direction. It is
 * hidden from assistive technology, which would read the user's own words
 * back over the dictation. */
export const LiveText: React.FC<{ text: string }> = ({ text }) => {
  const clipRef = useRef<HTMLSpanElement>(null);
  const wordsRef = useRef<HTMLSpanElement>(null);
  // The direction the words run in, while some of them are clipped: the
  // fade is on the edge they leave by.
  const [clipped, setClipped] = useState<"ltr" | "rtl" | null>(null);
  const shown = lastChars(text);

  const measure = useCallback(() => {
    const clip = clipRef.current;
    const words = wordsRef.current;
    if (!clip || !words) return;
    const direction =
      getComputedStyle(clip).direction === "rtl" ? "rtl" : "ltr";
    setClipped(words.offsetWidth > clip.offsetWidth ? direction : null);
  }, []);

  useLayoutEffect(measure, [measure, shown]);

  // The font can load, and the window change, under the same words.
  useEffect(() => {
    const clip = clipRef.current;
    const words = wordsRef.current;
    if (!clip || !words) return;
    const observer = new ResizeObserver(measure);
    observer.observe(clip);
    observer.observe(words);
    return () => observer.disconnect();
  }, [measure]);

  return (
    <span
      ref={clipRef}
      className="ov-live-text"
      data-clipped={clipped ?? undefined}
      aria-hidden="true"
    >
      <span ref={wordsRef} className="ov-live-words">
        {shown}
      </span>
    </span>
  );
};
