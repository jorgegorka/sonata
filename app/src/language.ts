export const LANGUAGES = ["en", "es", "fr", "de"] as const;
export type Language = (typeof LANGUAGES)[number];
export type LanguageChoice = Language | "system";

const NAMES: Record<Language, string> = { en: "English", es: "Español", fr: "Français", de: "Deutsch" };

export function isLanguage(code: string): code is Language {
  return (LANGUAGES as readonly string[]).includes(code);
}

export function nativeName(code: string): string {
  return isLanguage(code) ? NAMES[code] : NAMES.en;
}
