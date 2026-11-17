import { describe, expect, it } from "vitest";
import {
  afterFailure,
  afterRead,
  INITIAL_TERMINAL,
  showsReadFailed,
  TERMINAL_COLS,
  TERMINAL_ROWS,
} from "./terminal";

describe("terminal state", () => {
  it("matches the headless panes", () => {
    expect([TERMINAL_COLS, TERMINAL_ROWS]).toEqual([120, 40]);
  });

  it("shows the screen while the bot runs, and says so when it does not", () => {
    const shown = afterRead(INITIAL_TERMINAL, "\u001b[1m❯\u001b[0m");
    expect(shown).toEqual({ screen: "\u001b[1m❯\u001b[0m", running: true, failed: false });
    expect(afterRead(shown, null)).toEqual({ screen: null, running: false, failed: false });
  });

  it("keeps the last screen when a read fails", () => {
    const shown = afterRead(INITIAL_TERMINAL, "screen");
    expect(afterFailure(shown)).toEqual({ screen: "screen", running: true, failed: true });
    expect(afterRead(afterFailure(shown), "next")).toEqual({ screen: "next", running: true, failed: false });
  });

  it("says the last screen is shown only when there is one", () => {
    const shown = afterRead(INITIAL_TERMINAL, "screen");
    expect(showsReadFailed(afterFailure(shown))).toBe(true);
    expect(showsReadFailed(shown)).toBe(false);

    const failedFirst = afterFailure(INITIAL_TERMINAL);
    expect(failedFirst).toEqual({ screen: null, running: null, failed: true });
    expect(showsReadFailed(failedFirst)).toBe(false);

    const failedStopped = afterFailure(afterRead(shown, null));
    expect(failedStopped).toEqual({ screen: null, running: false, failed: true });
    expect(showsReadFailed(failedStopped)).toBe(false);
  });
});
