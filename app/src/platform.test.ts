import { describe, expect, it } from "vitest";
import { platformStrings } from "./platform";
import en from "../locales/en.json";

describe("desktop platform wording", () => {
  it("preserves Mac wording and switches Linux shortcuts and setup text", () => {
    expect(platformStrings(en, true).browser.pick).toContain("⇧⌘C");
    expect(platformStrings(en, false).browser.pick).toContain("Shift+Ctrl+C");
    expect(platformStrings(en, false).setup.intro).toContain("computer");
    expect(platformStrings(en, false).notifications.hiddenBody).toContain("taskbar");
    expect(platformStrings(en, false).menu.settings).toBe(en.menu.settings);
    expect(en.setup.intro).toContain("Mac");
  });
});
