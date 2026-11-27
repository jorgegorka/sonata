import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { avatarTextColor } from "./colors";
import { TERMINAL_THEME } from "./terminal";

export interface OmarchyPalette {
  mode: "light" | "dark";
  background: string;
  panel: string;
  text: string;
  muted: string;
  border: string;
  accent: string;
  mine: string;
  red: string;
  green: string;
  yellow: string;
  orange: string;
  console_bg: string;
  console_text: string;
}

export interface ConsoleColors {
  background: string;
  foreground: string;
}

const TOKENS = [
  "--bg",
  "--panel",
  "--text",
  "--muted",
  "--border",
  "--accent",
  "--accent-text",
  "--mine",
  "--error",
  "--needs-bg",
  "--needs-border",
  "--finished-bg",
  "--finished-border",
  "--shadow",
  "--card-shadow",
  "--ok",
  "--busy",
  "--alert",
  "--waits",
  "--console-bg",
  "--console-text",
] as const;

const MIDNIGHT_CONSOLE: ConsoleColors = {
  background: TERMINAL_THEME.background,
  foreground: TERMINAL_THEME.foreground,
};

function rgb(hex: string): [number, number, number] {
  return [parseInt(hex.slice(1, 3), 16), parseInt(hex.slice(3, 5), 16), parseInt(hex.slice(5, 7), 16)];
}

function mix(base: string, tint: string, amount: number): string {
  const from = rgb(base);
  const to = rgb(tint);
  const mixed = from.map((channel, i) => Math.round(channel + (to[i] - channel) * amount));
  return `#${mixed.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
}

export function cssVars(palette: OmarchyPalette): Record<(typeof TOKENS)[number], string> {
  const dark = palette.mode !== "light";
  return {
    "--bg": palette.background,
    "--panel": palette.panel,
    "--text": palette.text,
    "--muted": palette.muted,
    "--border": palette.border,
    "--accent": palette.accent,
    "--accent-text": avatarTextColor(palette.accent),
    "--mine": palette.mine,
    "--error": palette.red,
    "--needs-bg": mix(palette.background, palette.yellow, 0.18),
    "--needs-border": palette.yellow,
    "--finished-bg": mix(palette.background, palette.green, dark ? 0.18 : 0.12),
    "--finished-border": mix(palette.border, palette.green, 0.4),
    "--shadow": dark ? "rgba(0, 0, 0, 0.55)" : "rgba(0, 0, 0, 0.14)",
    "--card-shadow": dark ? "rgba(0, 0, 0, 0.35)" : "rgba(0, 0, 0, 0.06)",
    "--ok": palette.green,
    "--busy": palette.yellow,
    "--alert": palette.red,
    "--waits": palette.orange,
    "--console-bg": palette.console_bg,
    "--console-text": palette.console_text,
  };
}

let console: ConsoleColors | null = null;
const listeners = new Set<(colors: ConsoleColors) => void>();

export function consoleColors(): ConsoleColors {
  return console ?? MIDNIGHT_CONSOLE;
}

export function onConsoleColors(listener: (colors: ConsoleColors) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function applyOmarchyTheme(palette: OmarchyPalette | null, root: HTMLElement = document.documentElement): void {
  if (!palette) {
    for (const name of TOKENS) root.style.removeProperty(name);
    root.style.removeProperty("color-scheme");
  } else {
    for (const [name, value] of Object.entries(cssVars(palette))) root.style.setProperty(name, value);
    root.style.setProperty("color-scheme", palette.mode === "light" ? "light" : "dark");
  }
  console = palette ? { background: palette.console_bg, foreground: palette.console_text } : null;
  const colors = consoleColors();
  for (const listener of listeners) listener(colors);
}

export function readOmarchyTheme(): Promise<OmarchyPalette | null> {
  return invoke<OmarchyPalette | null>("omarchy_theme");
}

export function onOmarchyTheme(handler: (palette: OmarchyPalette | null) => void): Promise<() => void> {
  return listen<OmarchyPalette | null>("omarchy-theme", (event) => handler(event.payload));
}

// Listen before the first read, so a theme switch that lands while the read is in flight wins.
export function installOmarchyTheme(root?: HTMLElement): void {
  let seen = false;
  void onOmarchyTheme((palette) => {
    seen = true;
    applyOmarchyTheme(palette, root);
  })
    .then(() => readOmarchyTheme())
    .then((palette) => {
      if (!seen) applyOmarchyTheme(palette, root);
    })
    .catch(() => {});
}
