import { describe, expect, it } from "vitest";
import { isLanguage, LANGUAGES, nativeName } from "./language";

describe("language", () => {
  it("knows the four languages and names each in itself", () => {
    expect(LANGUAGES).toEqual(["en", "es", "fr", "de"]);
    expect(LANGUAGES.map(nativeName)).toEqual(["English", "Español", "Français", "Deutsch"]);
    expect(isLanguage("pt")).toBe(false);
    expect(nativeName("pt")).toBe("English");
  });
});
