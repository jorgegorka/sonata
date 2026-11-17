import type { PageState, Placement } from "./api";
import { SIDEBAR_RAIL_WIDTH } from "./sidebar";

export const MIN_CHAT_WIDTH = 200;
export const MIN_PANEL_WIDTH = 320;
export const DEFAULT_PANEL_WIDTH = 480;

export interface BrowserRequest {
  url: string;
  seq: number;
}

export interface BrowserState {
  open: boolean;
  request: BrowserRequest | null;
  url: string;
  title: string;
  loading: boolean;
  stalled: boolean;
  width: number;
  lastSeq: number;
  picking: boolean;
}

export type BrowserAction =
  | { type: "open"; url: string }
  | { type: "page"; page: PageState }
  | { type: "close" }
  | { type: "hide" }
  | { type: "toggle" }
  | { type: "stalled" }
  | { type: "picking"; on: boolean }
  | { type: "resize"; width: number; windowWidth: number; sidebarWidth?: number };

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export const WIDTH_KEY = "browserPanelWidth";

export function initialBrowser(storage: StorageLike | null): BrowserState {
  const width = readWidth(storage) ?? DEFAULT_PANEL_WIDTH;
  return {
    open: false,
    request: null,
    url: "",
    title: "",
    loading: false,
    stalled: false,
    width,
    lastSeq: 0,
    picking: false,
  };
}

export function browserReducer(state: BrowserState, action: BrowserAction): BrowserState {
  switch (action.type) {
    case "open": {
      const seq = state.lastSeq + 1;
      const request = { url: action.url, seq };
      return { ...state, open: true, request, url: action.url, title: "", loading: true, stalled: false, lastSeq: seq };
    }
    case "page":
      if (!state.open) return state;
      const { url, title, loading } = action.page;
      return { ...state, url, title, loading, stalled: false };
    case "close":
      return { ...state, open: false, request: null, picking: false };
    case "hide":
      if (!state.request && !state.picking) return state;
      return { ...state, request: null, picking: false };
    case "toggle":
      return { ...state, open: !state.open, request: null, picking: state.open ? false : state.picking };
    case "stalled":
      if (!state.open || !state.loading) return state;
      return { ...state, loading: false, stalled: true };
    case "picking":
      return { ...state, picking: action.on };
    case "resize":
      return { ...state, width: clampWidth(action.width, action.windowWidth, action.sidebarWidth) };
  }
}

export function maxPanelWidth(windowWidth: number, sidebarWidth = SIDEBAR_RAIL_WIDTH): number {
  return windowWidth - sidebarWidth - MIN_CHAT_WIDTH;
}

export function clampWidth(width: number, windowWidth: number, sidebarWidth = SIDEBAR_RAIL_WIDTH): number {
  const max = Math.max(MIN_PANEL_WIDTH, maxPanelWidth(windowWidth, sidebarWidth));
  return Math.round(Math.min(Math.max(width, MIN_PANEL_WIDTH), max));
}

export function dragWidth(startWidth: number, startX: number, x: number): number {
  return startWidth + startX - x;
}

export function readWidth(storage: StorageLike | null): number | null {
  try {
    const raw = storage?.getItem(WIDTH_KEY);
    if (raw === null || raw === undefined) return null;
    const width = Number(raw);
    return Number.isFinite(width) && width >= MIN_PANEL_WIDTH ? width : null;
  } catch {
    return null;
  }
}

export function writeWidth(storage: StorageLike | null, width: number): void {
  try {
    storage?.setItem(WIDTH_KEY, String(width));
  } catch {
  }
}

export function panelStorage(): StorageLike | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

export function gridColumns(width: number, sidebarWidth = SIDEBAR_RAIL_WIDTH): string {
  return `${sidebarWidth}px minmax(0, 1fr) min(${width}px, calc(100% - ${sidebarWidth + MIN_CHAT_WIDTH}px))`;
}

export function placement(
  rect: { left: number; top: number; right: number; bottom: number },
  viewport: { width: number; height: number },
): Placement {
  const x = Math.round(rect.left);
  const y = Math.round(rect.top);
  return {
    x,
    y,
    width: Math.round(rect.right) - x,
    height: Math.round(rect.bottom) - y,
    viewport_width: Math.round(viewport.width),
    viewport_height: Math.round(viewport.height),
  };
}

export function samePlacement(a: Placement, b: Placement): boolean {
  return (
    a.x === b.x &&
    a.y === b.y &&
    a.width === b.width &&
    a.height === b.height &&
    a.viewport_width === b.viewport_width &&
    a.viewport_height === b.viewport_height
  );
}

export function pickButton(state: { picking: boolean; url: string; pickAvailable: boolean }): {
  label: string;
  disabled: boolean;
} {
  if (state.picking) return { label: "browser.stopPick", disabled: false };
  return {
    label: state.pickAvailable ? "browser.pick" : "browser.pickUnavailable",
    disabled: !state.url || !state.pickAvailable,
  };
}

export function panelTitle(state: Pick<BrowserState, "title" | "url">): string | null {
  const title = state.title.trim();
  if (title) return title;
  try {
    return new URL(state.url).hostname || null;
  } catch {
    return null;
  }
}
