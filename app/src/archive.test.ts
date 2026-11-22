import { describe, expect, it } from "vitest";
import type { BotView, SessionView, ThreadSummary } from "./api";
import { archivedDate, archivedGroups, archivedThreads } from "./archive";

const bot = (id: string): BotView => ({ project_id: "default",
  id,
  handle: id,
  display_name: id,
  color: "#2563eb",
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status: "offline",
});

const session = (id: string, botId: string, archivedAt: string | null, number = 1): SessionView => ({ project_id: "default",
  id,
  bot_id: botId,
  number,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "t",
  last_activity_at: "t",
  archived_at: archivedAt,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "offline",
});

const thread = (id: string, a: string | null, b: string | null): ThreadSummary => ({
  id,
  participant_a: "b1",
  participant_b: "b2",
  session_a: a,
  session_b: b,
  last_message_at: "t",
});

describe("archivedGroups", () => {
  it("lists archived sessions by bot in the sidebar's order, the latest archived first", () => {
    const bots = [bot("b2"), bot("b1"), bot("b3")];
    const sessions = [
      session("old", "b1", "2026-10-01T10:00:00.000Z", 1),
      session("new", "b1", "2026-10-14T10:00:00.000Z", 2),
      session("tie", "b1", "2026-10-14T10:00:00.000Z", 3),
      session("live", "b1", null, 4),
      session("other", "b2", "2026-10-02T10:00:00.000Z"),
      session("kept", "b3", null),
    ];
    const groups = archivedGroups(bots, sessions).map((g) => [g.bot.id, g.sessions.map((s) => s.id)]);
    expect(groups).toEqual([
      ["b2", ["other"]],
      ["b1", ["tie", "new", "old"]],
    ]);
  });

  it("is empty when nothing is archived", () => {
    expect(archivedGroups([bot("b1")], [session("a", "b1", null)])).toEqual([]);
  });
});

describe("archivedThreads", () => {
  it("lists the bot threads with an archived end", () => {
    const sessions = [session("s1", "b1", null), session("s2", "b2", "2026-10-14T10:00:00.000Z")];
    const threads = [thread("t1", "s1", "s2"), thread("t2", "s1", null), thread("t3", "s2", "gone")];
    expect(archivedThreads(threads, sessions).map((t) => t.id)).toEqual(["t1", "t3"]);
  });
});

describe("archivedDate", () => {
  it("is the day the session was archived, in the user's language", () => {
    const archived = session("a", "b1", "2026-10-14T10:00:00.000Z");
    expect(archivedDate(archived, "en")).toBe("Oct 14, 2026");
    expect(archivedDate(archived, "de")).toBe("14.10.2026");
    expect(archivedDate(session("b", "b1", null), "en")).toBe("");
  });
});
