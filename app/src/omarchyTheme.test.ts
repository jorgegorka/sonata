// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from "vitest";
import { accentTextColor } from "./colors";
import { applyOmarchyTheme, consoleColors, cssVars, onConsoleColors, type OmarchyPalette } from "./omarchyTheme";
import { TERMINAL_THEME } from "./terminal";

const tokyo: OmarchyPalette = {
  mode: "dark",
  background: "#1a1b26",
  panel: "#24283b",
  text: "#a9b1d6",
  muted: "#565f89",
  border: "#414868",
  accent: "#7aa2f7",
  mine: "#292e42",
  red: "#f7768e",
  green: "#9ece6a",
  yellow: "#e0af68",
  orange: "#eb927b",
  console_bg: "#0e0e14",
  console_text: "#c0caf5",
};

const latte: OmarchyPalette = {
  mode: "light",
  background: "#eff1f5",
  panel: "#dce0e8",
  text: "#4c4f69",
  muted: "#9ca0b0",
  border: "#acb0be",
  accent: "#1e66f5",
  mine: "#ccd0da",
  red: "#d20f39",
  green: "#40a02b",
  yellow: "#df8e1d",
  orange: "#d84e2b",
  console_bg: "#d7d8dc",
  console_text: "#4c4f69",
};

afterEach(() => applyOmarchyTheme(null));

describe("omarchy theme", () => {
  it("paints Tokyo Night onto the app's tokens", () => {
    const vars = cssVars(tokyo);
    expect(vars["--bg"]).toBe("#1a1b26");
    expect(vars["--panel"]).toBe("#24283b");
    expect(vars["--text"]).toBe("#a9b1d6");
    expect(vars["--muted"]).toBe("#565f89");
    expect(vars["--border"]).toBe("#414868");
    expect(vars["--accent"]).toBe("#7aa2f7");
    expect(vars["--accent-text"]).toBe(accentTextColor("#7aa2f7"));
    expect(vars["--mine"]).toBe("#292e42");
    expect(vars["--error"]).toBe("#f7768e");
    expect(vars["--alert"]).toBe("#f7768e");
    expect(vars["--ok"]).toBe("#9ece6a");
    expect(vars["--busy"]).toBe("#e0af68");
    expect(vars["--waits"]).toBe("#eb927b");
    expect(vars["--needs-border"]).toBe("#e0af68");
    expect(vars["--needs-bg"]).toBe("#3e3632");
    expect(vars["--shadow"]).toBe("rgba(0, 0, 0, 0.55)");
    expect(vars["--card-shadow"]).toBe("rgba(0, 0, 0, 0.35)");
    expect(vars["--console-bg"]).toBe("#0e0e14");
    expect(vars["--console-text"]).toBe("#c0caf5");
  });

  it("paints a light theme with the lighter shadow", () => {
    const vars = cssVars(latte);
    expect(vars["--bg"]).toBe("#eff1f5");
    expect(vars["--accent"]).toBe("#1e66f5");
    expect(vars["--accent-text"]).toBe(accentTextColor("#1e66f5"));
    expect(vars["--needs-bg"]).toBe("#ecdfce");
    expect(vars["--shadow"]).toBe("rgba(0, 0, 0, 0.14)");
    expect(vars["--card-shadow"]).toBe("rgba(0, 0, 0, 0.06)");
    expect(vars["--console-bg"]).toBe("#d7d8dc");
    expect(vars["--console-text"]).toBe("#4c4f69");
  });

  it("sets the tokens inline and clears them when the theme goes away", () => {
    applyOmarchyTheme(tokyo);
    expect(document.documentElement.style.getPropertyValue("--bg")).toBe("#1a1b26");
    expect(document.documentElement.style.getPropertyValue("--accent")).toBe("#7aa2f7");
    expect(document.documentElement.style.getPropertyValue("color-scheme")).toBe("dark");
    expect(consoleColors()).toEqual({ background: "#0e0e14", foreground: "#c0caf5" });

    applyOmarchyTheme(null);
    expect(document.documentElement.style.getPropertyValue("--bg")).toBe("");
    expect(document.documentElement.style.getPropertyValue("color-scheme")).toBe("");
    expect(consoleColors()).toEqual({ background: TERMINAL_THEME.background, foreground: TERMINAL_THEME.foreground });
  });

  it("tells an open terminal about the new console colors", () => {
    const seen: { background: string; foreground: string }[] = [];
    const stop = onConsoleColors((colors) => seen.push(colors));
    applyOmarchyTheme(latte);
    applyOmarchyTheme(null);
    stop();
    applyOmarchyTheme(tokyo);
    expect(seen).toEqual([
      { background: "#d7d8dc", foreground: "#4c4f69" },
      { background: TERMINAL_THEME.background, foreground: TERMINAL_THEME.foreground },
    ]);
  });
});
