import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { BriefcaseIcon } from "@phosphor-icons/react/dist/csr/Briefcase";
import { ChatsCircleIcon } from "@phosphor-icons/react/dist/csr/ChatsCircle";
import { CodeIcon } from "@phosphor-icons/react/dist/csr/Code";
import { DotsThreeIcon } from "@phosphor-icons/react/dist/csr/DotsThree";
import { EnvelopeSimpleIcon } from "@phosphor-icons/react/dist/csr/EnvelopeSimple";
import { FileTextIcon } from "@phosphor-icons/react/dist/csr/FileText";
import { FireIcon } from "@phosphor-icons/react/dist/csr/Fire";
import { GaugeIcon } from "@phosphor-icons/react/dist/csr/Gauge";
import { RobotIcon } from "@phosphor-icons/react/dist/csr/Robot";
import { TextAaIcon } from "@phosphor-icons/react/dist/csr/TextAa";
import { TimerIcon } from "@phosphor-icons/react/dist/csr/Timer";
import { TrophyIcon } from "@phosphor-icons/react/dist/csr/Trophy";
import type { Icon } from "@phosphor-icons/react/dist/lib/types";
import {
  commands,
  events,
  type AppCategory,
  type HeatmapDay,
  type InsightsSummary,
} from "@/bindings";
import { DescriptionTooltip } from "../../ui/SettingContainer";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingsPage } from "../../ui/SettingsPage";
import {
  CATEGORY_LABEL_KEYS,
  monthLabelColumns,
  parseLocalDate,
  timeSaved,
  toWeeks,
  TYPING_WPM,
} from "./insightsModel";

const CATEGORY_ICONS: Record<AppCategory, Icon> = {
  ai_prompts: RobotIcon,
  personal_messages: ChatsCircleIcon,
  work_messages: BriefcaseIcon,
  emails: EnvelopeSimpleIcon,
  documents: FileTextIcon,
  coding: CodeIcon,
  other: DotsThreeIcon,
};

const HEATMAP_LEVEL_CLASSES = [
  "bg-mid-gray/15",
  "bg-logo-primary/30",
  "bg-logo-primary/55",
  "bg-logo-primary/80",
  "bg-logo-primary",
] as const;

const useFormatters = () => {
  const { i18n } = useTranslation();
  return useMemo(() => {
    const locale = i18n.language;
    return {
      number: new Intl.NumberFormat(locale),
      days: new Intl.NumberFormat(locale, {
        style: "unit",
        unit: "day",
        unitDisplay: "long",
      }),
      fullDate: new Intl.DateTimeFormat(locale, { dateStyle: "medium" }),
      month: new Intl.DateTimeFormat(locale, { month: "short" }),
      weekday: new Intl.DateTimeFormat(locale, { weekday: "short" }),
    };
  }, [i18n.language]);
};

const StatTile: React.FC<{
  icon: Icon;
  label: string;
  value: string;
  tooltip?: string;
}> = ({ icon: TileIcon, label, value, tooltip }) => (
  <section className="flex min-w-0 flex-col gap-1 rounded-lg border border-mid-gray/20 bg-background p-4">
    {/* Two lines high in every tile, so a label that wraps keeps the values
        level. The info icon flows with the label and wraps with its last
        word, leaving a long single word the full width. */}
    <div className="flex min-h-8 items-center gap-1.5 text-mid-gray">
      <TileIcon size={16} aria-hidden="true" className="shrink-0" />
      <div className="min-w-0 text-balance text-xs leading-4 font-medium">
        <h2 className="inline">{label}</h2>
        {tooltip && (
          <span className="ms-1 inline-flex h-4 items-center align-top">
            <DescriptionTooltip description={tooltip} position="bottom" />
          </span>
        )}
      </div>
    </div>
    <p className="text-3xl font-semibold tabular-nums">{value}</p>
  </section>
);

const AppUsage: React.FC<{ summary: InsightsSummary }> = ({ summary }) => {
  const { t } = useTranslation();
  const format = useFormatters();
  const total = summary.categorized_dictations;

  if (total === 0) {
    return (
      <p className="px-4 py-6 text-center text-sm text-mid-gray">
        {t("insights.apps.empty")}
      </p>
    );
  }

  return (
    <div className="space-y-3 p-4">
      <ul className="space-y-3">
        {summary.categories.map(({ category, count }) => {
          const CategoryIcon = CATEGORY_ICONS[category];
          const percent = Math.round((count * 100) / total);
          const label = t(CATEGORY_LABEL_KEYS[category]);
          return (
            <li
              key={category}
              className="grid grid-cols-[minmax(0,10rem)_1fr_auto] items-center gap-3"
            >
              <span className="flex min-w-0 items-center gap-2 text-sm">
                <CategoryIcon
                  size={18}
                  aria-hidden="true"
                  className="shrink-0 text-mid-gray"
                />
                <span className="truncate">{label}</span>
              </span>
              <span className="h-2 overflow-hidden rounded-full bg-mid-gray/15">
                <span
                  className="block h-full rounded-full bg-logo-primary"
                  style={{ width: `${Math.max(percent, 1)}%` }}
                />
              </span>
              <span className="w-24 text-end text-sm tabular-nums">
                {percent}%
                <span className="ms-2 text-xs text-mid-gray">
                  {format.number.format(count)}
                </span>
              </span>
            </li>
          );
        })}
      </ul>
      <p className="flex justify-between border-t border-mid-gray/20 pt-3 text-sm">
        <span className="text-mid-gray">{t("insights.apps.totalApps")}</span>
        <span className="font-medium tabular-nums">
          {format.number.format(summary.apps_used)}
        </span>
      </p>
    </div>
  );
};

const WEEKDAY_LABEL_ROWS = new Set([1, 3, 5]);

const Heatmap: React.FC<{ days: HeatmapDay[] }> = ({ days }) => {
  const { t } = useTranslation();
  const format = useFormatters();
  const weeks = toWeeks(days);
  const monthColumns = monthLabelColumns(weeks);
  // The first column starts on a Sunday, so its dates name each weekday row.
  const weekdayLabels = (weeks[0] ?? []).map((day) =>
    format.weekday.format(parseLocalDate(day.date)),
  );

  return (
    <div className="space-y-2">
      <div
        role="img"
        aria-label={t("insights.streak.heatmapLabel")}
        className="flex gap-2 overflow-x-auto"
      >
        <div className="grid shrink-0 grid-rows-[1rem_repeat(7,0.75rem)] gap-[3px] text-[10px] leading-3 text-mid-gray">
          <span />
          {weekdayLabels.map((label, row) => (
            <span key={row}>{WEEKDAY_LABEL_ROWS.has(row) ? label : ""}</span>
          ))}
        </div>
        <div className="flex gap-[3px]">
          {weeks.map((week, column) => {
            const start = parseLocalDate(week[0].date);
            const showMonth = monthColumns.has(column);
            return (
              <div
                key={week[0].date}
                className="grid grid-rows-[1rem_repeat(7,0.75rem)] gap-[3px]"
              >
                <span className="w-3 overflow-visible whitespace-nowrap text-[10px] leading-3 text-mid-gray">
                  {showMonth ? format.month.format(start) : ""}
                </span>
                {week.map((day) => (
                  <span
                    key={day.date}
                    data-level={day.level}
                    title={t("insights.streak.cellTitle", {
                      date: format.fullDate.format(parseLocalDate(day.date)),
                      value: format.number.format(day.count),
                    })}
                    className={`size-3 rounded-[3px] ${HEATMAP_LEVEL_CLASSES[day.level] ?? HEATMAP_LEVEL_CLASSES[0]}`}
                  />
                ))}
              </div>
            );
          })}
        </div>
      </div>
      <div className="flex items-center justify-end gap-1 text-[10px] text-mid-gray">
        <span className="me-1">{t("insights.streak.less")}</span>
        {HEATMAP_LEVEL_CLASSES.map((levelClass) => (
          <span
            key={levelClass}
            aria-hidden="true"
            className={`size-3 rounded-[3px] ${levelClass}`}
          />
        ))}
        <span className="ms-1">{t("insights.streak.more")}</span>
      </div>
    </div>
  );
};

const Streak: React.FC<{ summary: InsightsSummary }> = ({ summary }) => {
  const { t } = useTranslation();
  const format = useFormatters();
  const stats: { icon: Icon; label: string; days: number }[] = [
    {
      icon: FireIcon,
      label: t("insights.streak.current"),
      days: summary.streaks.current,
    },
    {
      icon: TrophyIcon,
      label: t("insights.streak.longest"),
      days: summary.streaks.longest,
    },
  ];

  return (
    <div className="space-y-4 p-4">
      <div className="grid grid-cols-2 gap-4">
        {stats.map(({ icon: StatIcon, label, days }) => (
          <div key={label} className="flex items-center gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-logo-primary/15 text-accent-text">
              <StatIcon size={18} weight="fill" aria-hidden="true" />
            </span>
            <div className="min-w-0">
              <p className="text-xs text-mid-gray">{label}</p>
              <p className="text-lg font-semibold tabular-nums">
                {format.days.format(days)}
              </p>
            </div>
          </div>
        ))}
      </div>
      <Heatmap days={summary.heatmap} />
    </div>
  );
};

export const InsightsView: React.FC<{ summary: InsightsSummary }> = ({
  summary,
}) => {
  const { t } = useTranslation();
  const format = useFormatters();
  const saved = timeSaved(summary.minutes_saved);

  return (
    <SettingsPage label={t("sidebar.insights")}>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <StatTile
          icon={GaugeIcon}
          label={t("insights.wpm.title")}
          value={
            summary.words_per_minute === null
              ? "–"
              : format.number.format(summary.words_per_minute)
          }
          tooltip={t("insights.wpm.formula")}
        />
        <StatTile
          icon={TextAaIcon}
          label={t("insights.words.title")}
          value={format.number.format(summary.total_words)}
        />
        <StatTile
          icon={TimerIcon}
          label={t("insights.timeSaved.title")}
          value={t(`insights.timeSaved.value.${saved.unit}`, {
            hours: format.number.format(saved.hours),
            minutes: format.number.format(saved.minutes),
          })}
          tooltip={t("insights.timeSaved.formula", {
            wpm: format.number.format(TYPING_WPM),
          })}
        />
      </div>

      <SettingsGroup title={t("insights.apps.title")}>
        <AppUsage summary={summary} />
      </SettingsGroup>

      <SettingsGroup title={t("insights.streak.title")}>
        <Streak summary={summary} />
      </SettingsGroup>
    </SettingsPage>
  );
};

const stopListener = async (stopListening: () => void) => {
  try {
    await stopListening();
  } catch (error) {
    console.error("Failed to stop insights listener:", error);
  }
};

export const InsightsSettings: React.FC = () => {
  const { t } = useTranslation();
  const [summary, setSummary] = useState<InsightsSummary | null>(null);
  const [failed, setFailed] = useState(false);
  const latestRequest = useRef(0);

  const load = useCallback(async () => {
    const request = ++latestRequest.current;
    const result = await commands.getInsights();
    if (request !== latestRequest.current) return;
    if (result.status === "ok") {
      setSummary(result.data);
      setFailed(false);
    } else {
      console.error("Failed to load insights:", result.error);
      setFailed(true);
    }
  }, []);

  useEffect(() => {
    void load();
    const unlisten = events.insightsUpdated
      .listen(() => void load())
      .catch((error) => {
        console.error("Failed to listen for insights updates:", error);
        return undefined;
      });
    return () => {
      void unlisten.then((stopListening) => {
        if (stopListening) return stopListener(stopListening);
      });
    };
  }, [load]);

  if (failed && !summary) {
    return (
      <SettingsPage label={t("sidebar.insights")}>
        <p className="px-4 py-6 text-center text-sm text-mid-gray">
          {t("insights.error")}
        </p>
      </SettingsPage>
    );
  }

  return summary ? <InsightsView summary={summary} /> : null;
};
