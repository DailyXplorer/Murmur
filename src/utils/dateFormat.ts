/**
 * Format a Unix timestamp as a localized date and time string, or return the
 * input unchanged when it does not parse
 * @param timestamp - Unix timestamp in seconds (as string)
 * @param locale - BCP 47 language tag (e.g., 'en', 'es', 'fr')
 * @returns Formatted date string
 */
export const formatDateTime = (timestamp: string, locale: string): string => {
  try {
    const timestampMs = parseInt(timestamp, 10) * 1000;
    const date = new Date(timestampMs);

    if (isNaN(date.getTime())) {
      return timestamp;
    }

    return new Intl.DateTimeFormat(locale, {
      year: "numeric",
      month: "long",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  } catch (error) {
    console.error("Failed to format date:", error);
    return timestamp;
  }
};
