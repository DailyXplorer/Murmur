import type { InsightsSummary } from "@/bindings";

/** The side of the share card, in pixels. */
export const SHARE_SIZE = 1080;

/** The public website, printed at the foot of the card. */
export const SHARE_URL = "murmur-desktop.vercel.app";

/**
 * Everything the share card may show, and nothing else: numbers and the
 * activity level of each day. The card never receives the full summary, so
 * app categories, app counts, dictation counts and any text stay off it by
 * construction.
 */
export interface ShareStats {
  totalWords: number;
  minutesSaved: number;
  /** Words per minute over the last 30 days; `null` before any measurable
   * dictation. */
  wpm: number | null;
  streaks: { current: number; longest: number };
  /** Sunday-first days, as in the Insights heatmap. Counts are left out. */
  heatmap: { date: string; level: number }[];
}

export const toShareStats = (summary: InsightsSummary): ShareStats => ({
  totalWords: summary.total_words,
  minutesSaved: summary.minutes_saved,
  wpm: summary.words_per_minute,
  streaks: {
    current: summary.streaks.current,
    longest: summary.streaks.longest,
  },
  heatmap: summary.heatmap.map(({ date, level }) => ({ date, level })),
});

/** Sharing starts once there is something to share. */
export const canShare = (summary: InsightsSummary): boolean =>
  summary.total_words > 0;
