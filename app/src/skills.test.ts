import i18next from "i18next";
import { describe, expect, it } from "vitest";
import type { BotView } from "./api";
import en from "../locales/en.json";
import es from "../locales/es.json";
import { canSave, reloadsSkills, sameIds, toggleId, usedByKey } from "./skills";

describe("skills", () => {
  it("reload when Master adds a skill or creates or changes a bot, whose skills' \"used by\" lines change", () => {
    expect(reloadsSkills({ type: "skills_changed" })).toBe(true);
    expect(reloadsSkills({ type: "bot_added", bot: {} as BotView })).toBe(true);
    expect(reloadsSkills({ type: "bot_updated", bot: {} as BotView })).toBe(true);
    expect(reloadsSkills({ type: "paused", paused: true })).toBe(false);
  });

  it("toggles an id, keeping each once", () => {
    expect(toggleId([], "a")).toEqual(["a"]);
    expect(toggleId(["a", "b"], "a")).toEqual(["b"]);
    expect(toggleId(["a", "a"], "b")).toEqual(["a", "b"]);
  });

  it("compares id sets regardless of order and repeats", () => {
    expect(sameIds(["a", "b"], ["b", "a"])).toBe(true);
    expect(sameIds(["a", "a"], ["a"])).toBe(true);
    expect(sameIds(["a"], ["a", "b"])).toBe(false);
  });

  it("needs a name and a description to save", () => {
    expect(canSave({ name: " ", description: "x", body: "" })).toBe(false);
    expect(canSave({ name: "x", description: " ", body: "" })).toBe(false);
    expect(canSave({ name: "x", description: "y", body: "" })).toBe(true);
  });

  it("picks the used-by text", () => {
    expect(usedByKey([])).toBe("skills.unused");
    expect(usedByKey(["a"])).toBe("skills.usedBy");
  });

  it("says used by in the singular for one bot and in the plural for more", async () => {
    const t = await i18next.createInstance().init({
      lng: "es",
      fallbackLng: "en",
      resources: { en: { translation: en }, es: { translation: es } },
      interpolation: { escapeValue: false },
    });
    expect(t(usedByKey(["a"]), { bots: "@a", count: 1 })).toBe("La usa @a");
    expect(t(usedByKey(["a", "b"]), { bots: "@a, @b", count: 2 })).toBe("La usan @a, @b");
    expect(t(usedByKey(["a"]), { bots: "@a", count: 1, lng: "en" })).toBe("Used by @a");
  });
});
