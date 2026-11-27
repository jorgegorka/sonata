import { describe, expect, it } from "vitest";
import { EMPTY_DRAFT, withDraft } from "./drafts";

describe("withDraft", () => {
  it("keeps one draft per key", () => {
    const drafts = withDraft(withDraft({}, "s1", { text: "hi", files: [] }), "s2", { text: "", files: ["/a.md"] });
    expect(drafts).toEqual({ s1: { text: "hi", files: [] }, s2: { text: "", files: ["/a.md"] } });
  });

  it("drops a draft that became empty", () => {
    expect(withDraft({ s1: { text: "hi", files: [] } }, "s1", EMPTY_DRAFT)).toEqual({});
  });
});
