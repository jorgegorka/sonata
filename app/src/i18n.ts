import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import de from "../locales/de.json";
import en from "../locales/en.json";
import es from "../locales/es.json";
import fr from "../locales/fr.json";
import { platformStrings } from "./platform";

void i18n.use(initReactI18next).init({
  resources: Object.fromEntries(Object.entries({ en, es, fr, de }).map(([code, strings]) => [code, { translation: platformStrings(strings) }])),
  lng: "en",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;
