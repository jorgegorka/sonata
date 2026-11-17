import { describe, expect, it } from "vitest";
import type { PickBoxState } from "../api";
import { boxKey, boxView, trimComment } from "./box";

const key = (o: Partial<{ key: string; shiftKey: boolean; isComposing: boolean; keyCode: number }>) =>
  boxKey({ key: "a", shiftKey: false, isComposing: false, keyCode: 0, ...o });

describe("boxKey", () => {
  it("sends on Enter", () => expect(key({ key: "Enter", keyCode: 13 })).toBe("send"));
  it("makes a new line on Shift+Enter", () => expect(key({ key: "Enter", shiftKey: true })).toBe("newline"));
  it("never sends while an input method composes", () => {
    expect(key({ key: "Enter", isComposing: true })).toBe("pass");
    expect(key({ key: "Enter", keyCode: 229 })).toBe("pass");
  });
  it("closes on Escape unless composing", () => {
    expect(key({ key: "Escape" })).toBe("close");
    expect(key({ key: "Escape", isComposing: true })).toBe("pass");
    expect(key({ key: "Escape", keyCode: 229 })).toBe("pass");
  });
  it("passes other keys", () => expect(key({ key: "x" })).toBe("pass"));
});

const strings = {
  hint: "h", placeholder: "p", keys: "k", send: "Send", sending: "Sending…", failed: "Not sent.", noTarget: "No chat.",
  tooLong: "Too long.",
};
const state = (o: Partial<PickBoxState> = {}): PickBoxState => ({
  target: "To @feature · Login fix", strings, status: "idle", generation: 1, ...o,
});

describe("boxView", () => {
  it("can send a comment to a target", () => {
    expect(boxView(state(), "fix it")).toEqual({
      target: "To @feature · Login fix", canSend: true, status: null, alert: false,
    });
  });
  it("cannot send without a target, and says so", () => {
    expect(boxView(state({ target: null }), "fix it")).toEqual({
      target: "No chat.", canSend: false, status: null, alert: false,
    });
  });
  it("cannot send a blank comment", () => {
    expect(boxView(state(), " \n ").canSend).toBe(false);
    expect(boxView(state(), "").canSend).toBe(false);
  });
  it("cannot send while sending, and shows it", () => {
    expect(boxView(state({ status: "sending" }), "x")).toMatchObject({ canSend: false, status: "Sending…", alert: false });
  });
  it("shows a failure and allows sending again", () => {
    expect(boxView(state({ status: "failed" }), "x")).toMatchObject({ canSend: true, status: "Not sent.", alert: true });
  });
  it("cannot send over 4,000 characters once trimmed, and says why", () => {
    expect(boxView(state(), "a".repeat(4001))).toMatchObject({ canSend: false, status: "Too long.", alert: true });
    expect(boxView(state({ status: "failed" }), "😀".repeat(4001))).toMatchObject({ status: "Too long." });
    expect(boxView(state(), "a".repeat(4000))).toMatchObject({ canSend: true, status: null });
    expect(boxView(state(), `  ${"😀".repeat(4000)}\n`)).toMatchObject({ canSend: true, status: null });
    expect(boxView(state({ status: "sending" }), "a".repeat(4001)).status).toBe("Sending…");
  });
  it("trims what Rust trims, and only that", () => {
    expect(trimComment("\u0085 \t\u00a0\u3000why\u2029\u205f\n\u0085")).toBe("why");
    expect(trimComment("\ufeffwhy\ufeff")).toBe("\ufeffwhy\ufeff");
    expect(boxView(state(), "\u0085\u2028").canSend).toBe(false);
    expect(boxView(state(), "\ufeff").canSend).toBe(true);
    expect(boxView(state(), `\u0085${"a".repeat(4000)}\u0085`).canSend).toBe(true);
  });
  it("shows nothing before the strings arrive", () => {
    expect(boxView(state({ strings: null }), "x")).toEqual({ target: "", canSend: false, status: null, alert: false });
  });
});
