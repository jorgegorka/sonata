import { describe, expect, it } from "vitest";
import { formatReset } from "./limits";

const now = new Date(2026, 10, 5, 10, 0);

describe("formatReset", () => {
  it("a reset later today is the time only", () => {
    expect(formatReset(new Date(2026, 10, 5, 15, 0).toISOString(), now, "en")).toBe("15:00");
  });
  it("a reset on another day includes the date and the time", () => {
    const text = formatReset(new Date(2026, 10, 6, 15, 0).toISOString(), now, "en");
    expect(text).toContain("Nov");
    expect(text).toContain("6");
    expect(text).toContain("15:00");
  });
});
