import { describe, expect, it } from "vitest";
import {
  DEFAULT_PANEL_WIDTH,
  MIN_CHAT_WIDTH,
  MIN_PANEL_WIDTH,
  WIDTH_KEY,
  browserReducer,
  clampWidth,
  dragWidth,
  gridColumns,
  initialBrowser,
  panelTitle,
  pickButton,
  placement,
  readWidth,
  samePlacement,
  writeWidth,
  type BrowserState,
  type StorageLike,
} from "./browser";
import { SIDEBAR_RAIL_WIDTH } from "./sidebar";

const page = (url: string, title = "", loading = false) => ({ url, title, loading });

describe("browserReducer", () => {
  it("starts closed with the default width", () => {
    const state = initialBrowser(null);
    expect(state.open).toBe(false);
    expect(state.width).toBe(DEFAULT_PANEL_WIDTH);
    expect(state.request).toBeNull();
  });

  it("open asks for the page, numbering each request", () => {
    const first = browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" });
    expect(first.open).toBe(true);
    expect(first.request).toEqual({ url: "https://a.test/", seq: 1 });
    expect([first.url, first.title, first.loading]).toEqual(["https://a.test/", "", true]);
    const second = browserReducer(first, { type: "open", url: "https://b.test/" });
    expect(second.request).toEqual({ url: "https://b.test/", seq: 2 });
  });

  it("page copies what Rust reports", () => {
    const open = browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" });
    const loaded = browserReducer(open, { type: "page", page: page("https://a.test/x", "A", false) });
    expect([loaded.url, loaded.title, loaded.loading]).toEqual(["https://a.test/x", "A", false]);
    expect(loaded.request).toEqual(open.request);
  });

  it("page is ignored while closed", () => {
    const closed = initialBrowser(null);
    expect(browserReducer(closed, { type: "page", page: page("https://a.test/", "A") })).toBe(closed);
  });

  it("close hides the panel and keeps the page and the width, forgetting the pending request", () => {
    const open: BrowserState = { ...browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" }), width: 600 };
    const loaded = browserReducer(open, { type: "page", page: page("https://a.test/x", "A", false) });
    const closed = browserReducer(loaded, { type: "close" });
    expect(closed.open).toBe(false);
    expect(closed.request).toBeNull();
    expect([closed.url, closed.title, closed.loading, closed.width]).toEqual(["https://a.test/x", "A", false, 600]);
  });

  it("toggle shows the kept page back without asking for it again, and hides it again", () => {
    const open = browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" });
    const hidden = browserReducer(open, { type: "close" });
    const shown = browserReducer(hidden, { type: "toggle" });
    expect([shown.open, shown.request, shown.url]).toEqual([true, null, "https://a.test/"]);
    const again = browserReducer(shown, { type: "toggle" });
    expect([again.open, again.request, again.url]).toEqual([false, null, "https://a.test/"]);
  });

  it("toggle with no page yet shows an empty panel", () => {
    const shown = browserReducer(initialBrowser(null), { type: "toggle" });
    expect([shown.open, shown.request, shown.url, shown.loading]).toEqual([true, null, "", false]);
  });

  it("hide keeps the panel open with its page and width, but forgets the request and ends pick mode", () => {
    const open: BrowserState = { ...browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" }), width: 600 };
    const loaded = browserReducer(open, { type: "page", page: page("https://a.test/x", "A", false) });
    const hidden = browserReducer(browserReducer(loaded, { type: "picking", on: true }), { type: "hide" });
    expect([hidden.open, hidden.request, hidden.picking]).toEqual([true, null, false]);
    expect([hidden.url, hidden.title, hidden.loading, hidden.width]).toEqual(["https://a.test/x", "A", false, 600]);
  });

  it("hide changes nothing with no request and no pick mode", () => {
    const closed = initialBrowser(null);
    expect(browserReducer(closed, { type: "hide" })).toBe(closed);
    const shown = browserReducer(closed, { type: "toggle" });
    expect(browserReducer(shown, { type: "hide" })).toBe(shown);
  });

  it("open while hidden shows the panel and asks for the new page", () => {
    const hidden = browserReducer(browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" }), { type: "close" });
    const reopened = browserReducer(hidden, { type: "open", url: "https://b.test/" });
    expect(reopened.open).toBe(true);
    expect(reopened.request).toEqual({ url: "https://b.test/", seq: 2 });
  });
});

describe("placement", () => {
  it("rounds the placeholder's rect to whole points and carries the viewport's size", () => {
    expect(placement({ left: 579.6, top: 57.4, right: 1100, bottom: 720 }, { width: 1100, height: 688 })).toEqual({
      x: 580,
      y: 57,
      width: 520,
      height: 663,
      viewport_width: 1100,
      viewport_height: 688,
    });
  });

  it("tells identical placements apart from changed ones", () => {
    const a = { x: 1, y: 2, width: 3, height: 4, viewport_width: 10, viewport_height: 10 };
    expect(samePlacement(a, { ...a })).toBe(true);
    expect(samePlacement(a, { ...a, width: 5 })).toBe(false);
  });
});

describe("panelTitle", () => {
  it("shows the title, else the host, else nothing", () => {
    expect(panelTitle({ title: "A page", url: "https://a.test/x" })).toBe("A page");
    expect(panelTitle({ title: "  ", url: "https://a.test/x" })).toBe("a.test");
    expect(panelTitle({ title: "", url: "" })).toBeNull();
  });
});

describe("gridColumns", () => {
  it("gives the panel its width beside the rail but never squeezes the chat below its minimum", () => {
    expect(gridColumns(480)).toBe("56px minmax(0, 1fr) min(480px, calc(100% - 256px))");
  });
});

describe("the panel's width", () => {
  it("leaves room for the rail and the chat at the window's minimum width", async () => {
    const conf = await import("../src-tauri/tauri.conf.json");
    expect(SIDEBAR_RAIL_WIDTH + MIN_CHAT_WIDTH + MIN_PANEL_WIDTH).toBeLessThanOrEqual(conf.default.app.windows[0].minWidth);
  });

  it("is clamped between the panel's minimum and what leaves the chat its minimum beside the rail", () => {
    expect(clampWidth(100, 1100)).toBe(320);
    expect(clampWidth(900, 1100)).toBe(844);
    expect(clampWidth(500.4, 1100)).toBe(500);
    expect(clampWidth(600, 560)).toBe(320);
  });

  it("is stored clamped by resize, and nothing else changes", () => {
    const open = browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" });
    const resized = browserReducer(open, { type: "resize", width: 900, windowWidth: 1100 });
    expect(resized.width).toBe(844);
    expect({ ...resized, width: open.width }).toEqual(open);
  });

  it("grows when the divider moves left", () => {
    expect(dragWidth(520, 800, 700)).toBe(620);
    expect(dragWidth(520, 800, 900)).toBe(420);
  });
});

describe("remembering the width", () => {
  const storage = (value: string | null, fail = false): StorageLike & { written: Record<string, string> } => ({
    written: {},
    getItem() {
      if (fail) throw new Error("no storage");
      return value;
    },
    setItem(key, v) {
      if (fail) throw new Error("no storage");
      this.written[key] = v;
    },
  });

  it("reads a sensible stored width and nothing else", () => {
    expect(readWidth(storage("600"))).toBe(600);
    for (const bad of ["abc", "-5", "100", null]) expect(readWidth(storage(bad)), String(bad)).toBeNull();
    expect(readWidth(storage("600", true))).toBeNull();
    expect(readWidth(null)).toBeNull();
  });

  it("writes the width and swallows a storage that fails", () => {
    const ok = storage(null);
    writeWidth(ok, 600);
    expect(ok.written).toEqual({ [WIDTH_KEY]: "600" });
    expect(() => writeWidth(storage(null, true), 600)).not.toThrow();
    expect(() => writeWidth(null, 600)).not.toThrow();
  });

  it("starts with the stored width", () => {
    expect(initialBrowser(storage("600")).width).toBe(600);
    expect(initialBrowser(storage(null)).width).toBe(DEFAULT_PANEL_WIDTH);
  });
});

describe("a page that never arrives", () => {
  const open = () => browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" });

  it("stops loading and says so when the panel reports a stall", () => {
    const stalled = browserReducer(open(), { type: "stalled" });
    expect([stalled.loading, stalled.stalled]).toEqual([false, true]);
  });

  it("ignores a stall when nothing is loading", () => {
    const loaded = browserReducer(open(), { type: "page", page: page("https://a.test/", "A", false) });
    expect(browserReducer(loaded, { type: "stalled" })).toBe(loaded);
  });

  it("forgets the stall when the page reports again or another link opens", () => {
    const stalled = browserReducer(open(), { type: "stalled" });
    expect(browserReducer(stalled, { type: "page", page: page("https://a.test/", "A", true) }).stalled).toBe(false);
    expect(browserReducer(stalled, { type: "open", url: "https://b.test/" }).stalled).toBe(false);
  });
});

describe("pick mode", () => {
  const open = () => browserReducer(initialBrowser(null), { type: "open", url: "https://a.test/" });

  it("starts off and follows what Rust reports", () => {
    expect(initialBrowser(null).picking).toBe(false);
    const on = browserReducer(open(), { type: "picking", on: true });
    expect(on.picking).toBe(true);
    expect(browserReducer(on, { type: "picking", on: false }).picking).toBe(false);
  });

  it("ends when the panel hides", () => {
    const on = browserReducer(open(), { type: "picking", on: true });
    expect(browserReducer(on, { type: "close" }).picking).toBe(false);
    const hidden = browserReducer(on, { type: "toggle" });
    expect([hidden.open, hidden.picking]).toEqual([false, false]);
  });

  it("does not start when the panel shows", () => {
    const hidden = browserReducer(open(), { type: "close" });
    const shown = browserReducer(hidden, { type: "toggle" });
    expect([shown.open, shown.picking]).toEqual([true, false]);
  });
});

describe("the pick button", () => {
  const page = "https://a.test/";

  it("offers to pick with a page and a chat to send to", () => {
    expect(pickButton({ picking: false, url: page, pickAvailable: true })).toEqual({
      label: "browser.pick",
      disabled: false,
    });
  });

  it("is disabled while off before any page, or with no chat to send to", () => {
    expect(pickButton({ picking: false, url: "", pickAvailable: true })).toEqual({
      label: "browser.pick",
      disabled: true,
    });
    expect(pickButton({ picking: false, url: page, pickAvailable: false })).toEqual({
      label: "browser.pickUnavailable",
      disabled: true,
    });
  });

  it("can always stop picking, also with no chat to send to", () => {
    for (const pickAvailable of [true, false]) {
      expect(pickButton({ picking: true, url: page, pickAvailable })).toEqual({
        label: "browser.stopPick",
        disabled: false,
      });
    }
  });
});
