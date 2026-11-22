import { describe, expect, it } from "vitest";
import type { Bot, Provider } from "./api";
import { PROVIDERS, deletedAnyway, draftOf, editNotes, modelPlaceholderKey } from "./botForm";
import { availableProviders } from "./setup";

const bot = (provider: Bot["provider"]): Bot => ({ project_id: "default",
  id: "b1",
  handle: "upper",
  display_name: "Upper",
  color: "#2563eb",
  provider,
  model: null,
  instructions: "Capitalize.",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
});

describe("editNotes", () => {
  it("says nothing about a colour change", () => {
    expect(editNotes(bot("claude"), { ...draftOf(bot("claude")), color: "#16a34a" })).toEqual([]);
  });

  it("says a change reaches the bot with its next message", () => {
    expect(editNotes(bot("claude"), { ...draftOf(bot("claude")), instructions: "Shout." })).toEqual(["botForm.appliesNext"]);
    expect(editNotes(bot("claude"), { ...draftOf(bot("claude")), model: "  " })).toEqual([]);
  });

  it("a changed skill set reaches the CLI", () => {
    const withSkill = { ...bot("claude"), skill_ids: ["s1"] };
    expect(editNotes(withSkill, { ...draftOf(withSkill), skill_ids: ["s1", "s1"] })).toEqual([]);
    expect(editNotes(withSkill, { ...draftOf(withSkill), skill_ids: [] })).toEqual(["botForm.appliesNext"]);
  });

  it("warns that a grok bot or a switch starts a new conversation", () => {
    expect(editNotes(bot("grok"), { ...draftOf(bot("grok")), display_name: "Loud" })).toEqual([
      "botForm.appliesNext",
      "botForm.grokNewConversation",
    ]);
    expect(editNotes(bot("claude"), { ...draftOf(bot("claude")), provider: "grok" })).toEqual([
      "botForm.appliesNext",
      "botForm.providerSwitch",
    ]);
  });
});

describe("deletedAnyway", () => {
  it("is true only when moving the folder to the Trash failed", () => {
    expect(deletedAnyway({ code: "trashFailed", params: { path: "/x", error: "e" } })).toBe(true);
    expect(deletedAnyway({ code: "skillTrashFailed", params: { path: "/x", error: "e" } })).toBe(true);
    expect(deletedAnyway({ code: "botNotFound", params: {} })).toBe(false);
    expect(deletedAnyway(new Error("x"))).toBe(false);
  });
});

describe("codex", () => {
  it("offers codex after claude, which stays the default", () => {
    expect(PROVIDERS.slice(0, 3)).toEqual(["claude", "codex", "grok"]);
    const found = (p: Provider) => ({ provider: p, path: `/bin/${p}`, version: null, logged_in: null });
    expect(availableProviders([found("grok"), found("codex"), found("claude")], PROVIDERS)[0]).toBe("claude");
    expect(availableProviders([found("grok"), found("codex")], PROVIDERS)).toEqual(["codex", "grok"]);
  });

  it("an edit of a codex bot keeps its conversation", () => {
    const b = bot("codex");
    expect(editNotes(b, { ...draftOf(b), instructions: "new" })).toEqual(["botForm.appliesNext"]);
  });
});

describe("opencode", () => {
  const found = (p: Provider) => ({ provider: p, path: `/bin/${p}`, version: null, logged_in: null });

  it("offers opencode last, claude stays the default", () => {
    expect(PROVIDERS).toEqual(["claude", "codex", "grok", "opencode"]);
    expect(availableProviders([found("opencode"), found("claude")], PROVIDERS)[0]).toBe("claude");
    expect(availableProviders([found("opencode")], PROVIDERS)).toEqual(["opencode"]);
  });

  it("asks for an opencode model as provider/model", () => {
    expect(modelPlaceholderKey("opencode")).toBe("createBot.modelPlaceholderOpencode");
    for (const p of ["claude", "codex", "grok"] as Provider[]) {
      expect(modelPlaceholderKey(p)).toBe("createBot.modelPlaceholder");
    }
  });

  it("an edit of an opencode bot keeps its conversation", () => {
    const b = bot("opencode");
    expect(editNotes(b, { ...draftOf(b), instructions: "new" })).toEqual(["botForm.appliesNext"]);
  });

  it("a new model starts a new opencode conversation", () => {
    const b = { ...bot("opencode"), model: "opencode-go/glm-5.3-flash" };
    expect(editNotes(b, { ...draftOf(b), model: "opencode-go/kimi-k3" })).toEqual([
      "botForm.appliesNext",
      "botForm.opencodeNewModel",
    ]);
    expect(editNotes(b, { ...draftOf(b), model: " opencode-go/glm-5.3-flash ", instructions: "new" })).toEqual([
      "botForm.appliesNext",
    ]);
    const claude = { ...bot("claude"), model: "sonnet" };
    expect(editNotes(claude, { ...draftOf(claude), model: "opus" })).toEqual(["botForm.appliesNext"]);
  });
});
