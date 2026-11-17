// @vitest-environment happy-dom
/// <reference types="vite/client" />
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PickBoxState } from "../api";
import page from "../../pickbox.html?raw";

const tauri = vi.hoisted(() => ({
  calls: [] as [string, unknown?][],
  hear: null as ((e: { payload: unknown }) => void) | null,
  answers: {} as Record<string, () => Promise<unknown>>,
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => {
    tauri.calls.push([command, args]);
    return tauri.answers[command]?.() ?? Promise.resolve(null);
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void) => {
    tauri.calls.push([`listen ${event}`]);
    tauri.hear = handler;
    return Promise.resolve(() => {});
  },
}));

const strings = {
  hint: "h", placeholder: "What should change here?", keys: "Enter to send", send: "Send", sending: "Sending…",
  failed: "Not sent.", noTarget: "No chat.", tooLong: "Too long.",
};
const state = (o: Partial<PickBoxState> = {}): PickBoxState => ({
  target: "To @feature · Login fix", strings, status: "idle", generation: 0, ...o,
});

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const comment = () => $<HTMLTextAreaElement>("comment");
const send = () => $<HTMLButtonElement>("send");
const status = () => $("status");
const hear = (s: PickBoxState) => tauri.hear!({ payload: s });
const type = (text: string) => {
  comment().value = text;
  comment().dispatchEvent(new Event("input", { bubbles: true }));
};
const key = (target: EventTarget, name: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
};
const commands = () => tauri.calls.map(([command]) => command);
const lastCall = () => tauri.calls[tauri.calls.length - 1];

const added: [EventTarget, string, EventListenerOrEventListenerObject][] = [];
const { addEventListener: addToDocument } = document;
const { addEventListener: addToWindow } = window;

async function load(read: () => Promise<unknown> = () => Promise.resolve(state())) {
  tauri.answers.browser_pick_box_state = read;
  vi.resetModules();
  await import("./main");
  await settle();
}

beforeEach(() => {
  tauri.calls = [];
  tauri.hear = null;
  tauri.answers = {};
  const markup = new DOMParser().parseFromString(page, "text/html").querySelector("main")!;
  document.body.replaceChildren(document.importNode(markup, true));
  document.addEventListener = function (this: Document, ...args: Parameters<Document["addEventListener"]>) {
    added.push([this, args[0], args[1]]);
    return addToDocument.apply(this, args);
  };
  window.addEventListener = function (this: Window, ...args: Parameters<Window["addEventListener"]>) {
    added.push([this, args[0], args[1]]);
    return addToWindow.apply(this, args);
  } as Window["addEventListener"];
});
afterEach(() => {
  for (const [target, type, listener] of added.splice(0)) target.removeEventListener(type, listener);
  document.addEventListener = addToDocument;
  window.addEventListener = addToWindow;
});

describe("the pick box page", () => {
  it("subscribes to its state before it reads it, and shows what it read", async () => {
    await load();
    expect(commands()).toEqual(["listen pickbox-state", "browser_pick_box_state"]);
    expect($("target").textContent).toBe("To @feature · Login fix");
    expect(comment().placeholder).toBe("What should change here?");
    expect(send().textContent).toBe("Send");
    expect(document.activeElement).toBe(comment());
  });

  it("shows the latest state: one heard while it was reading beats what it read", async () => {
    let answer: (s: PickBoxState) => void = () => {};
    await load(() => new Promise((resolve) => (answer = resolve)));
    hear(state({ target: "To @plan · New session", generation: 2 }));
    answer(state({ target: "To @feature · Login fix", generation: 1 }));
    await settle();
    expect($("target").textContent).toBe("To @plan · New session");
    hear(state({ target: null, generation: 2 }));
    expect($("target").textContent).toBe("No chat.");
    expect(send().disabled).toBe(true);
  });

  it("clears its text only when a new generation comes", async () => {
    await load();
    type("make it red");
    hear(state({ status: "sending" }));
    hear(state({ status: "failed" }));
    hear(state({ target: "To @plan · New session" }));
    expect(comment().value).toBe("make it red");
    hear(state({ generation: 1 }));
    expect(comment().value).toBe("");
  });

  it("sends the text on Enter, makes a line on Shift+Enter and closes on Esc", async () => {
    await load();
    type("make it red");
    expect(key(comment(), "Enter", { shiftKey: true }).defaultPrevented).toBe(false);
    expect(commands()).not.toContain("browser_pick_submit");
    expect(key(comment(), "Enter").defaultPrevented).toBe(true);
    expect(lastCall()).toEqual(["browser_pick_submit", { comment: "make it red" }]);
    expect(key(comment(), "Escape").defaultPrevented).toBe(true);
    expect(lastCall()).toEqual(["browser_pick_close", undefined]);
  });

  it("closes on Esc wherever its focus is, and sends on Enter only from the text", async () => {
    await load();
    type("make it red");
    send().focus();
    expect(key(send(), "Enter").defaultPrevented).toBe(false);
    expect(commands()).not.toContain("browser_pick_submit");
    expect(key(send(), "Escape").defaultPrevented).toBe(true);
    expect(lastCall()[0]).toBe("browser_pick_close");
    tauri.calls = [];
    comment().blur();
    expect(key(document.body, "Escape").defaultPrevented).toBe(true);
    expect(commands()).toEqual(["browser_pick_close"]);
    tauri.calls = [];
    expect(key(send(), "Escape", { isComposing: true }).defaultPrevented).toBe(false);
    expect(commands()).toEqual([]);
  });

  it("keeps typing in the text when the box is pressed outside it and its button", async () => {
    await load();
    type("make it red");
    const press = (el: Element) => {
      const e = new PointerEvent("pointerdown", { bubbles: true, cancelable: true });
      el.dispatchEvent(e);
      return e;
    };
    for (const el of [$("target"), document.querySelector("main")!, $("keys"), status()]) {
      send().focus();
      expect(document.activeElement).toBe(send());
      expect(press(el).defaultPrevented, el.className).toBe(true);
      expect(document.activeElement, el.className).toBe(comment());
    }
    send().focus();
    expect(press(send()).defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(send());
    expect(press(comment()).defaultPrevented).toBe(false);
  });

  it("says a comment Rust refused was not sent, until Rust tells it something new", async () => {
    await load();
    tauri.answers.browser_pick_submit = () => Promise.reject({ code: "browserFailed", params: {} });
    type("make it red");
    key(comment(), "Enter");
    await settle();
    expect([status().textContent, status().classList.contains("failed")]).toEqual(["Not sent.", true]);
    expect([comment().value, send().disabled]).toEqual(["make it red", false]);
    tauri.answers.browser_pick_submit = () => Promise.resolve(null);
    key(comment(), "Enter");
    expect(status().textContent).toBe("");
    tauri.answers.browser_pick_submit = () => Promise.reject({ code: "browserFailed", params: {} });
    key(comment(), "Enter");
    await settle();
    expect(status().textContent).toBe("Not sent.");
    hear(state());
    expect([status().textContent, status().classList.contains("failed")]).toEqual(["", false]);
  });

  it("says a comment is too long instead of only disabling Send", async () => {
    await load();
    type("a".repeat(4001));
    expect([status().textContent, status().classList.contains("failed"), send().disabled]).toEqual([
      "Too long.", true, true,
    ]);
    key(comment(), "Enter");
    expect(commands()).not.toContain("browser_pick_submit");
    type("a".repeat(4000));
    expect([status().textContent, status().classList.contains("failed"), send().disabled]).toEqual(["", false, false]);
  });

  it("sends nothing while it can't: no text, no chat, or one already sending", async () => {
    await load();
    key(comment(), "Enter");
    type("make it red");
    hear(state({ status: "sending" }));
    key(comment(), "Enter");
    hear(state({ target: null }));
    key(comment(), "Enter");
    expect(commands()).not.toContain("browser_pick_submit");
  });
});
