/**
 * The interface languages, in dropdown order (lower priority first). New ones
 * are not accepted; see CONTRIBUTING_TRANSLATIONS.md.
 */
export const LANGUAGE_METADATA: Record<
  string,
  {
    name: string;
    nativeName: string;
    priority?: number;
  }
> = {
  en: { name: "English", nativeName: "English", priority: 1 },
  fr: { name: "French", nativeName: "Français", priority: 2 },
  es: { name: "Spanish", nativeName: "Español", priority: 3 },
  de: { name: "German", nativeName: "Deutsch", priority: 4 },
  it: { name: "Italian", nativeName: "Italiano", priority: 5 },
  pt: { name: "Portuguese", nativeName: "Português", priority: 6 },
};
