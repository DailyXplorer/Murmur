/** "04:07" under an hour, "1:02:09" from one hour on. */
export const formatDuration = (milliseconds: number): string => {
  const totalSeconds = Math.max(0, Math.floor(milliseconds / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
};

/** "482 MB", "351 kB" under a megabyte or "1.2 GB" from a gigabyte on, in
 * the locale's style. */
export const formatSize = (bytes: number, locale: string): string => {
  if (bytes >= 999_500_000) {
    return new Intl.NumberFormat(locale, {
      style: "unit",
      unit: "gigabyte",
      maximumFractionDigits: 1,
    }).format(bytes / 1_000_000_000);
  }
  const kilobytes = Math.round(bytes / 1_000);
  return new Intl.NumberFormat(locale, {
    style: "unit",
    unit: kilobytes < 1_000 ? "kilobyte" : "megabyte",
    maximumFractionDigits: 0,
  }).format(kilobytes < 1_000 ? kilobytes : bytes / 1_000_000);
};

/** The meeting pill turns amber this long before the 3-hour cap. */
export const CAP_WARNING_MS = 5 * 60 * 1000;
