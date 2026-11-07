import { useMemo } from "react";
import { useTranslation } from "react-i18next";

/** The number, unit and date formats of Insights, in the app language. */
export const useFormatters = () => {
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
      hours: new Intl.NumberFormat(locale, {
        style: "unit",
        unit: "hour",
        unitDisplay: "narrow",
      }),
      minutes: new Intl.NumberFormat(locale, {
        style: "unit",
        unit: "minute",
        unitDisplay: "narrow",
      }),
      fullDate: new Intl.DateTimeFormat(locale, { dateStyle: "medium" }),
      month: new Intl.DateTimeFormat(locale, { month: "short" }),
      weekday: new Intl.DateTimeFormat(locale, { weekday: "short" }),
    };
  }, [i18n.language]);
};
