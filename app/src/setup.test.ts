import { describe, expect, it } from "vitest";
import type { ProviderStatus } from "./api";
import { availableProviders, providerState } from "./setup";

const status = (provider: "claude" | "grok", path: string | null, logged_in: boolean | null = null): ProviderStatus => ({
  provider,
  path,
  version: path ? "1.0" : null,
  logged_in,
});

describe("providerState", () => {
  it("tells ready, unchecked, not logged in and missing apart", () => {
    expect(providerState(status("claude", "/opt/homebrew/bin/claude", true))).toBe("ready");
    expect(providerState(status("grok", "/Users/u/.grok/bin/grok", null))).toBe("unchecked");
    expect(providerState(status("claude", "/opt/homebrew/bin/claude", false))).toBe("notLoggedIn");
    expect(providerState(status("claude", null))).toBe("missing");
  });
});

describe("availableProviders", () => {
  it("offers only the installed CLIs", () => {
    const found = [status("claude", null), status("grok", "/g/grok")];
    expect(availableProviders(found, ["claude", "grok"])).toEqual(["grok"]);
  });

  it("offers all of them until the first check answers", () => {
    expect(availableProviders(null, ["claude", "grok"])).toEqual(["claude", "grok"]);
  });
});
