import i18next, { type TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import en from "../locales/en.json";
import { errorMessage, failUnless, isCommandError, systemParams } from "./errors";

function english() {
  return i18next.createInstance().init({ lng: "en", resources: { en: { translation: en } }, interpolation: { escapeValue: false } });
}

describe("errorMessage", () => {
  it("translates a command error with its values", async () => {
    const t = await english();
    expect(errorMessage(t, { code: "fileTooLarge", params: { path: "/a/b.pdf", mb: "120" } })).toBe(
      "/a/b.pdf is 120 MB. The limit is 100 MB per file.",
    );
  });

  it("shows an unknown code inside the generic text", async () => {
    const t = await english();
    expect(errorMessage(t, { code: "nope", params: {} })).toBe("Something went wrong: nope");
  });

  it("shows anything else as text inside the generic text", async () => {
    const t = await english();
    expect(errorMessage(t, new Error("boom"))).toBe("Something went wrong: Error: boom");
    expect(isCommandError("boom")).toBe(false);
    expect(isCommandError({ code: "botNotFound", params: {} })).toBe(true);
  });

  it("translates a refusal code inside a system notice", () => {
    const t = ((key: string, opts?: { error?: string }) =>
      key === "errors.botStarting" ? "Starting!" : `${key}:${opts?.error ?? ""}`) as unknown as TFunction;
    expect(systemParams(t, { code: "botStarting", error: "raw" })).toEqual({ code: "botStarting", error: "Starting!" });
    expect(systemParams(t, { error: "raw" })).toEqual({ error: "raw" });
    expect(systemParams(t, null)).toEqual({});
  });
});

describe("failUnless", () => {
  it("passes on every error but the one with the code it expects", () => {
    const failed: unknown[] = [];
    const fail = failUnless("sessionNotFound", (e) => failed.push(e));
    fail({ code: "sessionNotFound", params: { id: "s1" } });
    fail({ code: "botNotFound" });
    fail(new Error("boom"));
    expect(failed).toEqual([{ code: "botNotFound" }, new Error("boom")]);
  });
});
