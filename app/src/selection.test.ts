import { describe, expect, it } from "vitest";
import type { SessionView } from "./api";
import { followsFirstSend, isChat } from "./selection";

const session = (id: string, botId: string): SessionView => ({ project_id: "default",
  id,
  bot_id: botId,
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "t",
  last_activity_at: "t",
  archived_at: null,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "offline",
});

describe("isChat", () => {
  it("is a session's chat, a blank chat or a bot thread", () => {
    expect(isChat({ kind: "session", id: "s1" })).toBe(true);
    expect(isChat({ kind: "newSession", botId: "b1" })).toBe(true);
    expect(isChat({ kind: "thread", id: "t1" })).toBe(true);
  });

  it("is not the board or any other page", () => {
    for (const kind of ["board", "new", "settings", "about", "archive", "tasks", "edit"]) {
      expect(isChat({ kind }), kind).toBe(false);
    }
    expect(isChat(null)).toBe(false);
  });
});

describe("followsFirstSend", () => {
  const made = session("s1", "b1");

  it("follows the new session while its bot's blank chat is still open", () => {
    expect(followsFirstSend({ kind: "newSession", botId: "b1" }, made)).toBe(true);
  });

  it("stays where the user went meanwhile", () => {
    expect(followsFirstSend({ kind: "newSession", botId: "b2" }, made)).toBe(false);
    expect(followsFirstSend({ kind: "session", id: "s0" }, made)).toBe(false);
    expect(followsFirstSend({ kind: "settings" }, made)).toBe(false);
    expect(followsFirstSend(null, made)).toBe(false);
  });
});
