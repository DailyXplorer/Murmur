import type { AppCategory, HeatmapDay } from "@/bindings";

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
