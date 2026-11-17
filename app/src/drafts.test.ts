import { describe, expect, it } from "vitest";
import { EMPTY_DRAFT, draftKey, moveDraft, newSessionKey, withDraft } from "./drafts";

describe("withDraft", () => {
  it("keeps one draft per key", () => {
    const drafts = withDraft(withDraft({}, "s1", { text: "hi", files: [] }), "s2", { text: "", files: ["/a.md"] });
    expect(drafts).toEqual({ s1: { text: "hi", files: [] }, s2: { text: "", files: ["/a.md"] } });
  });

  it("drops a draft that became empty", () => {
    expect(withDraft({ s1: { text: "hi", files: [] } }, "s1", EMPTY_DRAFT)).toEqual({});
  });
});

describe("draftKey", () => {
  it("is the session id, or new:<botId> for a blank chat, and none for a thread", () => {
    expect(draftKey({ kind: "session", id: "s1" })).toBe("s1");
    expect(draftKey({ kind: "newSession", botId: "b1" })).toBe("new:b1");
    expect(newSessionKey("b1")).toBe("new:b1");
    expect(draftKey({ kind: "thread", id: "t" })).toBeNull();
    expect(draftKey(null)).toBeNull();
  });
});

describe("moveDraft", () => {
  it("moves the blank chat's draft to the session the first send created", () => {
    const d = { text: "next", files: [] };
    const drafts = { "new:b1": d, "new:b2": { text: "x", files: [] } };
    expect(moveDraft(drafts, "b1", "s1")).toEqual({ s1: d, "new:b2": { text: "x", files: [] } });
  });

  it("changes nothing when there is no such draft", () => {
    const drafts = { s9: { text: "x", files: [] } };
    expect(moveDraft(drafts, "b1", "s1")).toBe(drafts);
  });
});
