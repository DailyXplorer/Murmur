import type { AppCategory, HeatmapDay } from "@/bindings";

export type EquivalenceUnit = "emails" | "pages" | "stories" | "novels";

interface EquivalenceTier {
  unit: EquivalenceUnit;
  wordsPerUnit: number;
  below: number;
}

const EQUIVALENCE_TIERS: readonly EquivalenceTier[] = [
  { unit: "emails", wordsPerUnit: 100, below: 3_000 },
  { unit: "pages", wordsPerUnit: 300, below: 30_000 },
  { unit: "stories", wordsPerUnit: 7_500, below: 90_000 },
  { unit: "novels", wordsPerUnit: 90_000, below: Infinity },
];

export interface WordEquivalence {
  unit: EquivalenceUnit;
  amount: number;
}

/** A relatable size for a word count, or `null` before the first dictation. */
export const wordEquivalence = (words: number): WordEquivalence | null => {
  if (words <= 0) return null;
  const tier =
    EQUIVALENCE_TIERS.find(({ below }) => words < below) ??
    EQUIVALENCE_TIERS[EQUIVALENCE_TIERS.length - 1];
  const raw = words / tier.wordsPerUnit;
  const amount = raw < 10 ? Math.round(raw * 10) / 10 : Math.round(raw);
  return { unit: tier.unit, amount: Math.max(amount, 0.1) };
};

/** The typing speed `minutes_saved` is measured against (`stats::TYPING_WPM`). */
export const TYPING_WPM = 40;

export interface TimeSaved {
  unit: "minutes" | "hours" | "hoursMinutes";
  hours: number;
  minutes: number;
}

/** Minutes below two hours, then hours with any leftover minutes. */
export const timeSaved = (totalMinutes: number): TimeSaved => {
  if (totalMinutes < 120) {
    return { unit: "minutes", hours: 0, minutes: totalMinutes };
  }
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return { unit: minutes === 0 ? "hours" : "hoursMinutes", hours, minutes };
};

export const CATEGORY_LABEL_KEYS: Record<AppCategory, string> = {
  ai_prompts: "insights.apps.categories.aiPrompts",
  personal_messages: "insights.apps.categories.personalMessages",
  work_messages: "insights.apps.categories.workMessages",
  emails: "insights.apps.categories.emails",
  documents: "insights.apps.categories.documents",
  coding: "insights.apps.categories.coding",
  other: "insights.apps.categories.other",
};

/** Parses the backend's `YYYY-MM-DD` as a local calendar date. */
export const parseLocalDate = (date: string): Date => {
  const [year, month, day] = date.split("-").map(Number);
  return new Date(year, month - 1, day);
};

/** A short month name needs about three cell widths. */
const MONTH_LABEL_SPAN = 3;

/**
 * Week columns that start a new month and are far enough from the next
 * month label for both names to fit.
 */
export const monthLabelColumns = (weeks: HeatmapDay[][]): Set<number> => {
  const starts = weeks.flatMap((week, column) => {
    if (column === 0) return [column];
    const month = parseLocalDate(week[0].date).getMonth();
    const previous = parseLocalDate(weeks[column - 1][0].date).getMonth();
    return month === previous ? [] : [column];
  });
  return new Set(
    starts.filter(
      (column, i) =>
        i === starts.length - 1 || starts[i + 1] - column >= MONTH_LABEL_SPAN,
    ),
  );
};

/** Groups Sunday-first days into week columns for the heatmap grid. */
export const toWeeks = (days: HeatmapDay[]): HeatmapDay[][] => {
  const weeks: HeatmapDay[][] = [];
  for (let i = 0; i < days.length; i += 7) {
    weeks.push(days.slice(i, i + 7));
  }
  return weeks;
};
