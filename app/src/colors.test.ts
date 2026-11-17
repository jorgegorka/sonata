import { describe, expect, it } from "vitest";
import { avatarTextColor } from "./colors";

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
