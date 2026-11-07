import React, { useLayoutEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { FireIcon } from "@phosphor-icons/react/dist/csr/Fire";
import { GaugeIcon } from "@phosphor-icons/react/dist/csr/Gauge";
import { TimerIcon } from "@phosphor-icons/react/dist/csr/Timer";
import { TrophyIcon } from "@phosphor-icons/react/dist/csr/Trophy";
import type { Icon } from "@phosphor-icons/react/dist/lib/types";
import murmurMark from "@/assets/murmur-mark.png";
import murmurMarkSilhouette from "@/assets/murmur-mark-silhouette.png";
import murmurTextLogo from "@/assets/murmur-text-logo.webp";
import { useFormatters } from "../insightsFormat";
import {
  monthLabelColumns,
  parseLocalDate,
  timeSaved,
  toWeeks,
} from "../insightsModel";
import { fitCard } from "./fitText";
import { SHARE_SIZE, SHARE_URL, type ShareStats } from "./shareStats";

/**
 * The picture a user shares: a square card of their Insights numbers.
 *
 * It is always the light card in the Murmur pink, whatever theme and accent
 * the app uses, so it looks the same wherever it lands. Every colour and
 * size is set here, not through the app's theme or Tailwind, for the same
 * reason. It is drawn at 1080 px and scaled for the preview.
 */

const BRAND = "Murmur";
const SEPARATOR = "\u2014";

const PINK = "250, 162, 202";
const TEXT = "#0f0f0f";
const HERO = "#382731";
const MID = "#808080";
const ACCENT_TEXT = "#895b70";
const LINE = "rgba(128, 128, 128, 0.22)";
const SANS =
  '"Instrument Sans Variable", "Instrument Sans", system-ui, sans-serif';
const SERIF = '"Faculty Glyphic", Georgia, serif';

const HEATMAP_LEVELS = [
  "rgba(128, 128, 128, 0.15)",
  `rgba(${PINK}, 0.30)`,
  `rgba(${PINK}, 0.55)`,
  `rgba(${PINK}, 0.80)`,
  `rgb(${PINK})`,
] as const;

const CELL = 21;
const GAP = 4;

const panel: React.CSSProperties = {
  background: "#ffffff",
  border: `1.5px solid ${LINE}`,
  borderRadius: 26,
};

const oneLine: React.CSSProperties = {
  display: "inline-block",
  whiteSpace: "nowrap",
};

const styles = {
  card: {
    position: "relative",
    boxSizing: "border-box",
    width: SHARE_SIZE,
    height: SHARE_SIZE,
    padding: "64px 76px 60px",
    display: "flex",
    flexDirection: "column",
    overflow: "hidden",
    background: "#fbfbfb",
    color: TEXT,
    fontFamily: SANS,
    fontSize: 16,
    fontWeight: 400,
    lineHeight: "normal",
    letterSpacing: 0,
    textAlign: "left",
    fontFeatureSettings: '"tnum" 0',
    WebkitFontSmoothing: "antialiased",
  },
  glow: {
    position: "absolute",
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
    background: `radial-gradient(700px 560px at 105% -8%, rgba(${PINK}, 0.42), transparent 70%), radial-gradient(640px 520px at -12% 108%, rgba(${PINK}, 0.26), transparent 70%)`,
  },
  silhouette: {
    position: "absolute",
    right: -120,
    top: -90,
    width: 560,
    height: 560,
    transform: "rotate(12deg)",
  },
  header: {
    position: "relative",
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    height: 64,
  },
  wordmark: { display: "block", height: 62 },
  pill: {
    fontSize: 26,
    fontWeight: 500,
    lineHeight: "32px",
    color: ACCENT_TEXT,
    border: `1.5px solid rgba(${PINK}, 0.7)`,
    background: `rgba(${PINK}, 0.14)`,
    borderRadius: 999,
    padding: "8px 24px",
  },
  hero: { position: "relative", marginTop: 34 },
  eyebrow: {
    margin: 0,
    fontFamily: SERIF,
    fontWeight: 400,
    fontSize: 44,
    lineHeight: "52px",
    color: ACCENT_TEXT,
  },
  // A fixed height, so a number that shrinks does not move what is below.
  heroNumber: {
    height: 255,
    display: "flex",
    alignItems: "center",
    paddingRight: 16,
    marginLeft: -8,
  },
  heroValue: {
    ...oneLine,
    fontSize: 250,
    fontWeight: 600,
    lineHeight: 1.02,
    letterSpacing: "-0.048em",
    color: HERO,
  },
  tiles: {
    position: "relative",
    display: "grid",
    gridTemplateColumns: "1fr 1fr",
    gap: 22,
    marginTop: 30,
  },
  tile: { ...panel, padding: "22px 32px 18px", minWidth: 0 },
  tileLabel: {
    display: "flex",
    alignItems: "center",
    gap: 12,
    color: MID,
    fontWeight: 500,
    lineHeight: "32px",
  },
  slot: { minWidth: 0, flex: 1 },
  tileValue: {
    margin: "4px 0 0",
    lineHeight: "87px",
    fontWeight: 600,
    letterSpacing: "-0.02em",
    height: 87,
  },
  streak: {
    ...panel,
    position: "relative",
    display: "flex",
    alignItems: "center",
    gap: 36,
    padding: "24px 32px",
    marginTop: 22,
  },
  streakStats: {
    display: "flex",
    flexDirection: "column",
    gap: 18,
    flex: "none",
    width: 260,
  },
  stat: { display: "flex", alignItems: "center", gap: 16 },
  statIcon: {
    width: 56,
    height: 56,
    borderRadius: "50%",
    background: `rgba(${PINK}, 0.17)`,
    color: ACCENT_TEXT,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    flex: "none",
  },
  statLabel: { margin: 0, color: MID, lineHeight: "28px", height: 28 },
  statValue: { margin: 0, fontWeight: 600, lineHeight: "42px", height: 42 },
  heatmap: { marginLeft: "auto", marginRight: 12 },
  months: {
    display: "grid",
    gridAutoFlow: "column",
    gridAutoColumns: CELL,
    columnGap: GAP,
    height: 20,
    marginBottom: GAP,
    fontSize: 18,
    lineHeight: "20px",
    color: MID,
  },
  month: { whiteSpace: "nowrap", overflow: "visible" },
  days: {
    display: "grid",
    gridAutoFlow: "column",
    gridTemplateRows: `repeat(7, ${CELL}px)`,
    gridAutoColumns: CELL,
    gap: GAP,
  },
  footer: {
    position: "relative",
    marginTop: "auto",
    paddingTop: 24,
    display: "flex",
    alignItems: "center",
    gap: 16,
    color: MID,
    lineHeight: "36px",
  },
  footerMark: { display: "block", height: 40 },
  footerText: { minWidth: 0, flex: 1 },
  brand: { fontWeight: 600, color: TEXT },
  separator: { margin: "0 2px", opacity: 0.5 },
} satisfies Record<string, React.CSSProperties>;

const Label: React.FC<{
  group: string;
  size: number;
  children: React.ReactNode;
}> = ({ group, size, children }) => (
  <div style={styles.slot}>
    <span data-fit={group} data-fit-max={size} style={oneLine}>
      {children}
    </span>
  </div>
);

const Tile: React.FC<{
  icon: Icon;
  label: string;
  value: string;
}> = ({ icon: TileIcon, label, value }) => (
  <section style={styles.tile}>
    <div style={styles.tileLabel}>
      <TileIcon size={30} weight="light" aria-hidden="true" />
      <Label group="tile-label" size={27}>
        {label}
      </Label>
    </div>
    <p style={styles.tileValue}>
      <span data-fit="tile-value" data-fit-max={78} style={oneLine}>
        {value}
      </span>
    </p>
  </section>
);

const Stat: React.FC<{ icon: Icon; label: string; value: string }> = ({
  icon: StatIcon,
  label,
  value,
}) => (
  <div style={styles.stat}>
    <span style={styles.statIcon}>
      <StatIcon size={30} weight="fill" aria-hidden="true" />
    </span>
    <div style={styles.slot}>
      <p style={styles.statLabel}>
        <span data-fit="streak-label" data-fit-max={23} style={oneLine}>
          {label}
        </span>
      </p>
      <p style={styles.statValue}>
        <span data-fit="streak-value" data-fit-max={36} style={oneLine}>
          {value}
        </span>
      </p>
    </div>
  </div>
);

const Heatmap: React.FC<{ days: ShareStats["heatmap"] }> = ({ days }) => {
  const format = useFormatters();
  // Counts are not part of the share stats; the month labels only need dates.
  const weeks = toWeeks(days.map((day) => ({ ...day, count: 0 })));
  const monthColumns = monthLabelColumns(weeks);

  return (
    <div style={styles.heatmap} aria-hidden="true">
      <div style={styles.months}>
        {weeks.map((week, column) => (
          <span key={week[0].date} style={styles.month}>
            {monthColumns.has(column)
              ? format.month.format(parseLocalDate(week[0].date))
              : ""}
          </span>
        ))}
      </div>
      <div style={styles.days}>
        {weeks.flat().map((day) => (
          <span
            key={day.date}
            style={{
              borderRadius: 5,
              background: HEATMAP_LEVELS[day.level] ?? HEATMAP_LEVELS[0],
            }}
          />
        ))}
      </div>
    </div>
  );
};

export const ShareCard: React.FC<{
  stats: ShareStats;
  ref?: React.Ref<HTMLDivElement>;
}> = ({ stats, ref }) => {
  const { t, i18n } = useTranslation();
  const format = useFormatters();
  const rootRef = useRef<HTMLDivElement>(null);
  const saved = timeSaved(stats.minutesSaved);

  const content = [
    stats.totalWords,
    stats.minutesSaved,
    stats.wpm,
    stats.streaks.current,
    stats.streaks.longest,
    i18n.language,
  ].join("|");

  // Fit again whenever the numbers or the language change, and once the
  // fonts are in: they change every width.
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const fit = () => fitCard(root);
    fit();
    let active = true;
    void document.fonts.ready.then(() => active && fit());
    document.fonts.addEventListener("loadingdone", fit);
    return () => {
      active = false;
      document.fonts.removeEventListener("loadingdone", fit);
    };
  }, [content]);

  const setRef = (node: HTMLDivElement | null) => {
    rootRef.current = node;
    if (typeof ref === "function") ref(node);
    else if (ref) ref.current = node;
  };

  return (
    <div ref={setRef} style={styles.card} lang={i18n.language}>
      <div style={styles.glow} />
      <img src={murmurMarkSilhouette} alt="" style={styles.silhouette} />

      <header style={styles.header}>
        <img src={murmurTextLogo} alt={BRAND} style={styles.wordmark} />
        <span style={styles.pill}>{t("sidebar.insights")}</span>
      </header>

      <section style={styles.hero}>
        <p style={styles.eyebrow}>{t("insights.share.card.wordsDictated")}</p>
        <div style={styles.heroNumber}>
          <span data-fit="hero" data-fit-max={250} style={styles.heroValue}>
            {format.number.format(stats.totalWords)}
          </span>
        </div>
      </section>

      <div style={styles.tiles}>
        <Tile
          icon={TimerIcon}
          label={t("insights.timeSaved.title")}
          value={t(`insights.timeSaved.value.${saved.unit}`, {
            hours: format.number.format(saved.hours),
            minutes: format.number.format(saved.minutes),
          })}
        />
        <Tile
          icon={GaugeIcon}
          label={t("insights.wpm.title")}
          value={stats.wpm === null ? "–" : format.number.format(stats.wpm)}
        />
      </div>

      <section style={styles.streak}>
        <div style={styles.streakStats}>
          <Stat
            icon={FireIcon}
            label={t("insights.streak.current")}
            value={format.days.format(stats.streaks.current)}
          />
          <Stat
            icon={TrophyIcon}
            label={t("insights.streak.longest")}
            value={format.days.format(stats.streaks.longest)}
          />
        </div>
        <Heatmap days={stats.heatmap} />
      </section>

      <footer style={styles.footer}>
        <img src={murmurMark} alt="" style={styles.footerMark} />
        <div style={styles.footerText}>
          <span data-fit="footer" data-fit-max={26} style={oneLine}>
            {t("insights.share.card.dictatedWith")}{" "}
            <b style={styles.brand}>{BRAND}</b>
            <span style={styles.separator}> {SEPARATOR} </span>
            {SHARE_URL}
          </span>
        </div>
      </footer>
    </div>
  );
};
