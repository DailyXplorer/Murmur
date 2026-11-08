import type {
  HistoryEntry,
  HistoryVersion,
  InsightsSummary,
} from "../../../src/bindings";

export const HOUR = 3600;
export const NOW = 1_760_000_000;
export const HISTORY: HistoryEntry[] = [
  {
    cleanup: "cleaned",
    engine: "cloud",
    fallback_reason: null,
    file_name: "murmur-1.wav",
    id: 3,
    raw_text: "um so the quarterly report is ready for review",
    saved: true,
    timestamp: NOW,
    title: "Quarterly report",
    transcription_text: "The quarterly report is ready for review.",
    version_count: 2,
  },
  {
    cleanup: null,
    engine: "cloud",
    fallback_reason: null,
    file_name: "murmur-2.wav",
    id: 2,
    raw_text: null,
    saved: false,
    timestamp: NOW - HOUR,
    title: "Meeting notes",
    transcription_text: "Move the design review to Thursday afternoon.",
    version_count: 0,
  },
  {
    cleanup: "failed",
    engine: "cloud",
    fallback_reason: null,
    file_name: "murmur-3.wav",
    id: 1,
    raw_text: "remember to water the plants",
    saved: false,
    timestamp: NOW - 26 * HOUR,
    title: "Reminder",
    transcription_text: "remember to water the plants",
    version_count: 0,
  },
];

/** The earlier transcripts of entry 3, newest first. */
export const VERSIONS: HistoryVersion[] = [
  {
    cleanup: null,
    engine: "local",
    fallback_reason: null,
    id: 31,
    produced_at: NOW - 2 * HOUR,
    raw_text: null,
    transcription_text: "The quarterly report is ready for a review.",
  },
  {
    cleanup: "cleaned",
    engine: "cloud",
    fallback_reason: null,
    id: 30,
    produced_at: NOW - 3 * HOUR,
    raw_text: "um the quarterly report ready for review",
    transcription_text: "The quarterly report is ready for review.",
  },
];

/** One entry per fallback reason, then one transcribed by the chosen engine. */
export const FALLBACK_HISTORY: HistoryEntry[] = (
  [
    ["cloud", "model_not_ready"],
    ["cloud", "local_failed"],
    ["cloud", "language_not_supported"],
    ["local", "cloud_limit_reached"],
    ["local", "cloud_failed"],
    ["local", null],
  ] as const
).map(([used, fallback_reason], index) => ({
  ...HISTORY[1],
  engine: used,
  fallback_reason,
  file_name: `murmur-fallback-${index}.wav`,
  id: 10 + index,
  timestamp: NOW - index * HOUR,
}));

export const INSIGHTS: InsightsSummary = {
  apps_used: 9,
  categories: [
    { category: "ai_prompts", count: 64 },
    { category: "work_messages", count: 31 },
    { category: "emails", count: 12 },
  ],
  categorized_dictations: 107,
  heatmap: Array.from({ length: 182 }, (_, index) => {
    const date = new Date(Date.UTC(2025, 3, 1) + index * 86_400_000);
    const count = index % 6 === 0 ? 0 : (index * 5) % 9;
    return {
      count,
      date: date.toISOString().slice(0, 10),
      level: count === 0 ? 0 : Math.min(4, Math.ceil(count / 2)),
    };
  }),
  minutes_saved: 273,
  streaks: { current: 6, longest: 21 },
  total_dictations: 412,
  total_words: 15_380,
  words_per_minute: 138,
};
