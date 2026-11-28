import { describe, expect, it } from "vitest";
import { accentTextColor, avatarTextColor } from "./colors";

describe("avatarTextColor", () => {
  it("keeps white on the darker preset colors", () => {
    for (const c of ["#2563eb", "#16a34a", "#dc2626", "#9333ea", "#ea580c", "#0891b2", "#db2777", "#0d9488", "#475569"]) {
      expect(avatarTextColor(c)).toBe("#ffffff");
    }
  });
  it("switches to dark on light colors", () => {
    expect(avatarTextColor("#ffffff")).toBe("#111827");
    expect(avatarTextColor("#FDE047")).toBe("#111827");
    expect(avatarTextColor("#ca8a04")).toBe("#111827");
  });
  it("falls back to white for values that are not #rrggbb", () => {
    expect(avatarTextColor("#000")).toBe("#ffffff");
    expect(avatarTextColor("blue")).toBe("#ffffff");
  });
});

describe("accentTextColor", () => {
  it("keeps normal text legible across light, dark, and midtone accents", () => {
    function luminance(hex: string) {
      const channels = [1, 3, 5].map((index) => {
        const channel = parseInt(hex.slice(index, index + 2), 16) / 255;
        return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      });
      return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
    }
    for (const background of ["#4b44d6", "#7a73ff", "#7aa2f7", "#1e66f5", "#808080", "#777777", "#ffffff", "#000000"]) {
      const values = [luminance(background), luminance(accentTextColor(background))].sort((a, b) => a - b);
      expect((values[1] + 0.05) / (values[0] + 0.05), background).toBeGreaterThanOrEqual(4.5);
    }
  });
});
