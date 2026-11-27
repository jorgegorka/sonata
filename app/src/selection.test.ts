import { describe, expect, it } from "vitest";
import { isChat } from "./selection";
describe("isChat", () => {
  it("only opens existing conversations", () => {
    expect(isChat({ kind: "session", id: "s1" })).toBe(true);
    expect(isChat({ kind: "thread", id: "t1" })).toBe(true);
    for (const kind of ["newSession", "newRun", "board", "settings", "archive"]) expect(isChat({ kind })).toBe(false);
    expect(isChat(null)).toBe(false);
  });
});
