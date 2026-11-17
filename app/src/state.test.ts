import { describe, expect, it } from "vitest";
import type { BotView, Message, SessionView, ThreadSummary } from "./api";
import {
  conversationOf,
  handleOf,
  botWaits,
  hasArchivedEnd,
  initialState,
  isArchived,
  isWaiting,
  reducer,
  removalCount,
  renamedTo,
  sessionLabel,
  sessionsOf,
  showsArchived,
  threadName,
  type Action,
  type AppState,
} from "./state";

const bot = (id: string): BotView => ({
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

const session = (id: string, botId = "b1", extra: Partial<SessionView> = {}): SessionView => ({
  id,
  bot_id: botId,
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "t",
  last_activity_at: "2026-10-07T10:00:00.000Z",
  archived_at: null,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "offline",
  ...extra,
});

const msg = (id: string, sender: string, recipient: string, status: Message["status"] = "delivered"): Message => ({
  id,
  thread_id: "th",
  sender,
  recipient,
  sender_session: null,
  recipient_session: null,
  kind: "text",
  body: id,
  attachments: [],
  system_code: null,
  system_params: null,
  status,
  chain_id: "c",
  hop: 0,
  in_reply_to: null,
  created_at: "2026-10-07T10:00:00.000Z",
  delivered_at: null,
  task_id: null,
  task_name: null,
  task_since: null,
  workflow_mark: null,
  workflow_request: false,
});

const toSession = (id: string, sessionId: string): Message => ({ ...msg(id, "user", "b1"), recipient_session: sessionId });
const fromSession = (id: string, sessionId: string): Message => ({ ...msg(id, "b1", "user"), sender_session: sessionId });
const ui = (event: Extract<Action, { type: "ui_event" }>["event"]): Action => ({ type: "ui_event", event });

describe("reducer", () => {
  it("applies an edit but keeps the bot's live status", () => {
    const s = reducer({ ...initialState, bots: [{ ...bot("b1"), status: "working" }] }, {
      type: "bot_updated",
      bot: { ...bot("b1"), display_name: "Renamed", config_version: 2 },
    });
    expect(s.bots[0]).toMatchObject({ display_name: "Renamed", config_version: 2, status: "working" });
  });

  it("files user messages under the recipient session and bot messages under the sender session", () => {
    let s = reducer(initialState, { type: "ui_event", event: { type: "message_added", message: toSession("m1", "s1") } });
    s = reducer(s, { type: "ui_event", event: { type: "message_added", message: fromSession("m2", "s1") } });
    expect(s.messages["s1"].map((m) => m.id)).toEqual(["m1", "m2"]);
  });

  it("replaces an updated message instead of duplicating it", () => {
    let s = reducer(initialState, { type: "ui_event", event: { type: "message_added", message: { ...toSession("m1", "s1"), status: "queued" } } });
    s = reducer(s, { type: "ui_event", event: { type: "message_updated", message: toSession("m1", "s1") } });
    expect(s.messages["s1"]).toHaveLength(1);
    expect(s.messages["s1"][0].status).toBe("delivered");
  });

  it("updates only the matching bot's status", () => {
    let s = reducer(initialState, { type: "bots_loaded", bots: [bot("b1"), bot("b2")] });
    s = reducer(s, { type: "ui_event", event: { type: "bot_status", bot_id: "b2", status: "working" } });
    expect(s.bots.map((b) => b.status)).toEqual(["offline", "working"]);
  });

  it("adds a created bot once, offline", () => {
    const created = { ...bot("b1"), status: "working" as const };
    let s = reducer(initialState, { type: "bot_created", bot: created });
    s = reducer(s, { type: "bot_created", bot: created });
    expect(s.bots).toHaveLength(1);
    expect(s.bots[0].status).toBe("offline");
  });

  it("a bot_updated event replaces the bot", () => {
    let s = reducer(initialState, { type: "bots_loaded", bots: [{ ...bot("b1"), status: "working" }, bot("b2")] });
    s = reducer(s, ui({ type: "bot_updated", bot: { ...bot("b1"), instructions: "New.", config_version: 2 } }));
    expect(s.bots[0]).toMatchObject({ id: "b1", instructions: "New.", config_version: 2, status: "working" });
    expect(s.bots[1]).toEqual(bot("b2"));
  });

  it("adds a bot Master created once", () => {
    const event = { type: "bot_added" as const, bot: bot("b1") };
    let s = reducer(initialState, { type: "ui_event", event });
    s = reducer(s, { type: "ui_event", event });
    expect(s.bots.map((b) => b.id)).toEqual(["b1"]);
  });

  it("keeps messages that arrived by event but are missing from a loaded list", () => {
    let s = reducer(initialState, { type: "ui_event", event: { type: "message_added", message: fromSession("m2", "s1") } });
    s = reducer(s, { type: "messages_loaded", key: "s1", messages: [toSession("m1", "s1")] });
    expect(s.messages["s1"].map((m) => m.id)).toEqual(["m1", "m2"]);
  });

  it("files bot-to-bot messages under their thread and moves the thread to the top", () => {
    let s = reducer(initialState, {
      type: "threads_loaded",
      threads: [{ id: "t-old", participant_a: "b1", participant_b: "b3", session_a: null, session_b: null, last_message_at: "t" }],
    });
    const m = { ...msg("m1", "b2", "b1"), thread_id: "t12", sender_session: "s2", recipient_session: "s1" };
    s = reducer(s, { type: "ui_event", event: { type: "message_added", message: m } });
    expect(s.messages["t12"].map((x) => x.id)).toEqual(["m1"]);
    expect(Object.keys(s.messages)).toEqual(["t12"]);
    expect(s.threads.map((t) => [t.id, t.participant_a, t.participant_b])).toEqual([
      ["t12", "b1", "b2"],
      ["t-old", "b1", "b3"],
    ]);
    expect([s.threads[0].session_a, s.threads[0].session_b]).toEqual(["s1", "s2"]);
  });

  it("keeps the skill library", () => {
    const skill = {
      id: "s1", slug: "audit", name: "Audit", description: "Audits.", source: "written" as const,
      created_at: "t", updated_at: "t", used_by: ["researcher"],
    };
    const state = reducer(initialState, { type: "skills_loaded", skills: [skill] });
    expect(state.skills).toEqual([skill]);
    expect(reducer(state, { type: "paused_loaded", paused: true }).skills).toEqual([skill]);
  });

  it("tracks the paused flag", () => {
    let s = reducer(initialState, { type: "paused_loaded", paused: true });
    expect(s.paused).toBe(true);
    s = reducer(s, { type: "ui_event", event: { type: "paused", paused: false } });
    expect(s.paused).toBe(false);
  });
});

describe("conversations", () => {
  it("are the session for user chats and the thread for bot-to-bot messages", () => {
    expect(conversationOf(toSession("m", "s1"))).toBe("s1");
    expect(conversationOf(fromSession("m", "s1"))).toBe("s1");
    expect(conversationOf({ ...msg("m", "b1", "b2"), thread_id: "t" })).toBe("t");
  });

  it("name bots by handle", () => {
    expect(handleOf([bot("b1")], "b1")).toBe("@b1");
    expect(handleOf([], "gone")).toBeNull();
  });
});

describe("sessions in the reducer", () => {
  const withSessions = (...list: SessionView[]) => reducer(initialState, { type: "sessions_loaded", sessions: list });

  it("loads the list", () => {
    expect(withSessions(session("s1")).sessions.map((x) => x.id)).toEqual(["s1"]);
  });

  it("adds a session once and replaces it when updated", () => {
    let s = reducer(initialState, ui({ type: "session_added", session: session("s1") }));
    s = reducer(s, ui({ type: "session_added", session: session("s1", "b1", { title: "again" }) }));
    expect(s.sessions).toHaveLength(1);
    s = reducer(s, ui({ type: "session_updated", session: session("s1", "b1", { title: "New" }) }));
    expect(s.sessions[0].title).toBe("New");
    s = reducer(s, ui({ type: "session_updated", session: session("s2") }));
    expect(s.sessions.map((x) => x.id)).toEqual(["s1", "s2"]);
  });

  it("keeps a known session's live status when it is added again", () => {
    let s = reducer(initialState, ui({ type: "session_added", session: session("s1") }));
    s = reducer(s, ui({ type: "session_status", session_id: "s1", bot_id: "b1", status: "working" }));
    s = reducer(s, ui({ type: "session_added", session: session("s1", "b1", { title: "Hi" }) }));
    expect(s.sessions.map((x) => [x.id, x.title, x.status])).toEqual([["s1", "Hi", "working"]]);
  });

  it("updates only the matching session's status, and ignores one it does not know", () => {
    const start = withSessions(session("s1"), session("s2"));
    const s = reducer(start, ui({ type: "session_status", session_id: "s2", bot_id: "b1", status: "working" }));
    expect(s.sessions.map((x) => x.status)).toEqual(["offline", "working"]);
    expect(reducer(start, ui({ type: "session_status", session_id: "zz", bot_id: "b1", status: "idle" }))).toBe(start);
  });

  it("drops a removed session, its chat, its threads and their messages", () => {
    const thread: ThreadSummary = { id: "t1", participant_a: "b1", participant_b: "b2", session_a: "s1", session_b: "s9", last_message_at: "t" };
    const other: ThreadSummary = { ...thread, id: "t2", session_a: "s5" };
    let s = { ...withSessions(session("s1"), session("s2")), threads: [thread, other] };
    s = reducer(s, ui({ type: "message_added", message: toSession("m1", "s1") }));
    s = reducer(s, ui({ type: "message_added", message: toSession("m2", "s2") }));
    s = { ...s, messages: { ...s.messages, t1: [msg("m3", "b1", "b2")], t2: [msg("m4", "b1", "b2")] } };
    s = reducer(s, ui({ type: "session_removed", session_id: "s1", bot_id: "b1", thread_ids: ["t1"] }));
    expect(s.sessions.map((x) => x.id)).toEqual(["s2"]);
    expect(s.threads.map((t) => t.id)).toEqual(["t2"]);
    expect(Object.keys(s.messages).sort()).toEqual(["s2", "t2"]);
  });

  it("clears opened_by on the sessions the removed one had opened", () => {
    const start = withSessions(session("s1"), session("s2", "b2", { opened_by: "s1" }), session("s3", "b2", { opened_by: "s2" }));
    const s = reducer(start, ui({ type: "session_removed", session_id: "s1", bot_id: "b1", thread_ids: [] }));
    expect(s.sessions.map((x) => [x.id, x.opened_by])).toEqual([["s2", null], ["s3", "s2"]]);
  });

  it("ignores late events for a removed session or one of its threads", () => {
    let s = withSessions(session("s1"), session("s2"));
    s = reducer(s, ui({ type: "session_removed", session_id: "s1", bot_id: "b1", thread_ids: ["t1"] }));
    const after = [
      ui({ type: "message_added", message: toSession("m1", "s1") }),
      ui({ type: "message_updated", message: fromSession("m1", "s1") }),
      ui({ type: "message_added", message: { ...msg("m2", "b2", "b1"), thread_id: "t1", sender_session: "s2", recipient_session: "s1" } }),
      ui({ type: "message_added", message: { ...msg("m3", "b2", "b1"), thread_id: "t1" } }),
      ui({ type: "session_added", session: session("s1") }),
      ui({ type: "session_updated", session: session("s1") }),
      ui({ type: "session_status", session_id: "s1", bot_id: "b1", status: "working" }),
      { type: "messages_loaded", key: "s1", messages: [toSession("m1", "s1")] } as Action,
    ];
    for (const action of after) expect(reducer(s, action), JSON.stringify(action).slice(0, 60)).toBe(s);
  });

  it("drops a deleted bot's sessions and chats, keeps its bot threads and clears opened_by", () => {
    const thread: ThreadSummary = { id: "t1", participant_a: "b1", participant_b: "b2", session_a: "s1", session_b: "s2", last_message_at: "t" };
    let s: AppState = {
      ...withSessions(session("s1"), session("s2", "b2", { opened_by: "s1" })),
      bots: [bot("b1"), bot("b2")],
      threads: [thread],
      messages: { s1: [toSession("m1", "s1")], s2: [], t1: [] },
    };
    s = reducer(s, { type: "bot_deleted", botId: "b1" });
    expect(s.sessions.map((x) => [x.id, x.opened_by])).toEqual([["s2", null]]);
    expect(Object.keys(s.messages).sort()).toEqual(["s2", "t1"]);
    expect(s.threads).toEqual([thread]);
    expect(reducer(s, ui({ type: "session_added", session: session("s1") }))).toBe(s);
  });

  it("keeps later messages in a deleted bot's bot threads and still drops those of its chats", () => {
    const thread: ThreadSummary = { id: "t1", participant_a: "b1", participant_b: "b2", session_a: "s1", session_b: "s2", last_message_at: "t" };
    const inThread = (id: string, status: Message["status"]): Message => ({
      ...msg(id, "b2", "b1", status),
      thread_id: "t1",
      sender_session: "s2",
      recipient_session: "s1",
    });
    let s: AppState = { ...withSessions(session("s1"), session("s2", "b2")), bots: [bot("b1"), bot("b2")], threads: [thread] };
    s = reducer(s, ui({ type: "message_added", message: inThread("m1", "queued") }));
    s = reducer(s, { type: "bot_deleted", botId: "b1" });
    s = reducer(s, ui({ type: "message_updated", message: inThread("m1", "cancelled") }));
    s = reducer(s, ui({ type: "message_added", message: { ...inThread("m2", "delivered"), sender: "b1", recipient: "b2", sender_session: "s1", recipient_session: "s2" } }));
    expect(s.messages["t1"].map((m) => [m.id, m.status])).toEqual([["m1", "cancelled"], ["m2", "delivered"]]);
    const late = reducer(s, ui({ type: "message_updated", message: fromSession("m3", "s1") }));
    expect(late).toBe(s);
  });

  it("leaves out loaded sessions that were removed meanwhile", () => {
    let s = withSessions(session("s1"), session("s2"));
    s = reducer(s, ui({ type: "session_removed", session_id: "s1", bot_id: "b1", thread_ids: [] }));
    s = reducer(s, { type: "sessions_loaded", sessions: [session("s1"), session("s2"), session("s3")] });
    expect(s.sessions.map((x) => x.id)).toEqual(["s2", "s3"]);
  });

  it("keeps sessions that arrived through events during the load and are not in the loaded list", () => {
    let s = withSessions(session("s1", "b1", { title: "old" }));
    s = reducer(s, ui({ type: "session_added", session: session("s3") }));
    s = reducer(s, ui({ type: "session_status", session_id: "s3", bot_id: "b1", status: "working" }));
    s = reducer(s, ui({ type: "session_added", session: session("s4") }));
    s = reducer(s, ui({ type: "session_removed", session_id: "s4", bot_id: "b1", thread_ids: [] }));
    s = reducer(s, { type: "sessions_loaded", sessions: [session("s1", "b1", { title: "new" }), session("s2")] });
    expect(s.sessions.map((x) => [x.id, x.title, x.status])).toEqual([
      ["s1", "new", "offline"],
      ["s2", null, "offline"],
      ["s3", null, "working"],
    ]);
  });

  it("still ignores event types it does not know", () => {
    const s = withSessions(session("s1"));
    expect(reducer(s, ui({ type: "from_the_future" } as never))).toBe(s);
  });
});

describe("sessionsOf", () => {
  it("lists a bot's sessions, latest activity first, ties by higher number", () => {
    const list = [
      session("a", "b1", { number: 1, last_activity_at: "2026-10-07T10:00:00.000Z" }),
      session("b", "b1", { number: 2, last_activity_at: "2026-10-07T10:00:00.000Z" }),
      session("c", "b1", { number: 3, last_activity_at: "2026-10-06T10:00:00.000Z" }),
      session("d", "b1", { number: 4, last_activity_at: "2026-10-08T10:00:00.000Z" }),
      session("x", "b2"),
    ];
    const copy = list.slice();
    expect(sessionsOf(list, "b1").map((x) => x.id)).toEqual(["d", "b", "a", "c"]);
    expect(list).toEqual(copy);
    expect(sessionsOf(list, "none")).toEqual([]);
  });
});

describe("sessionLabel", () => {
  const t = (key: string, o?: Record<string, string | number>) => `${key}|${JSON.stringify(o ?? {})}`;
  const bots = [bot("b1"), bot("b2")];

  it("is the title when there is one", () => {
    const s = session("s1", "b1", { title: "Plan", opened_by: "s2" });
    expect(sessionLabel(s, [s], bots, t)).toBe("Plan");
  });

  it("names the asker for a bot-opened session: its title, else its number", () => {
    const asker = session("s2", "b2", { number: 4, title: "Review" });
    const opened = session("s1", "b1", { opened_by: "s2" });
    expect(sessionLabel(opened, [opened, asker], bots, t)).toBe(t("sessions.openedBy", { handle: "@b2", label: "Review" }));
    const untitled = { ...asker, title: "  " };
    expect(sessionLabel(opened, [opened, untitled], bots, t)).toBe(
      t("sessions.openedBy", { handle: "@b2", label: t("sessions.numbered", { n: 4 }) }),
    );
  });

  it("is numbered otherwise, also when the asker is gone", () => {
    const plain = session("s1", "b1", { number: 3 });
    expect(sessionLabel(plain, [plain], bots, t)).toBe(t("sessions.numbered", { n: 3 }));
    const orphan = session("s1", "b1", { number: 3, opened_by: "gone" });
    expect(sessionLabel(orphan, [orphan], bots, t)).toBe(t("sessions.numbered", { n: 3 }));
    const asker = session("s2", "gone-bot");
    const opened = session("s5", "b1", { number: 5, opened_by: "s2" });
    expect(sessionLabel(opened, [opened, asker], bots, t)).toBe(t("sessions.numbered", { n: 5 }));
  });
});

describe("removalCount", () => {
  it("counts the other sessions a removal takes, recursively", () => {
    const list = [
      session("s1"),
      session("s2", "b2", { opened_by: "s1" }),
      session("s3", "b2", { opened_by: "s2" }),
      session("s4", "b2", { opened_by: "s1" }),
      session("s5", "b2"),
    ];
    expect(removalCount(list, "s1")).toBe(3);
    expect(removalCount(list, "s2")).toBe(1);
    expect(removalCount(list, "s5")).toBe(0);
    expect(removalCount(list, "gone")).toBe(0);
  });

  it("survives a loop", () => {
    const list = [session("a", "b1", { opened_by: "b" }), session("b", "b1", { opened_by: "a" })];
    expect(removalCount(list, "a")).toBe(1);
  });
});

describe("threadName", () => {
  const t = (key: string, o?: Record<string, string | number>) => (key === "thread.deletedBot" ? "(gone)" : `${key}|${JSON.stringify(o ?? {})}`);
  const bots = [bot("b1"), bot("b2"), bot("b3")];
  const thread = (a: string | null, b: string | null, pa = "b1", pb = "b2"): ThreadSummary => ({
    id: "t1",
    participant_a: pa,
    participant_b: pb,
    session_a: a,
    session_b: b,
    last_message_at: "t",
  });

  it("names each end by its handle and its session", () => {
    const list = [session("s1", "b1", { title: "MR plan" }), session("s2", "b2", { title: "Review" })];
    expect(threadName(thread("s1", "s2"), list, bots, t)).toBe("@b1 · MR plan ↔ @b2 · Review");
  });

  it("names only the handle of an untitled session the other end opened", () => {
    const list = [session("s1", "b1", { title: "MR plan" }), session("s2", "b2", { opened_by: "s1" })];
    expect(threadName(thread("s1", "s2"), list, bots, t)).toBe("@b1 · MR plan ↔ @b2");
  });

  it("labels an untitled session a third session opened", () => {
    const list = [
      session("s1", "b1", { title: "MR plan" }),
      session("s2", "b2", { opened_by: "s3" }),
      session("s3", "b3", { title: "Audit" }),
    ];
    expect(threadName(thread("s1", "s2"), list, bots, t)).toBe(
      `@b1 · MR plan ↔ @b2 · ${t("sessions.openedBy", { handle: "@b3", label: "Audit" })}`,
    );
  });

  it("names only the handle of an end whose session is not known, and a deleted bot as such", () => {
    expect(threadName(thread(null, "s9"), [], bots, t)).toBe("@b1 ↔ @b2");
    expect(threadName(thread("s1", "s2", "gone", "b2"), [session("s2", "b2", { number: 2 })], bots, t)).toBe(
      `(gone) ↔ @b2 · ${t("sessions.numbered", { n: 2 })}`,
    );
  });
});

describe("archived sessions", () => {
  const archived = (id: string, botId = "b1") => session(id, botId, { archived_at: "2026-10-14T10:00:00.000Z" });
  const thread = (a: string | null, b: string | null): ThreadSummary => ({
    id: "t1",
    participant_a: "b1",
    participant_b: "b2",
    session_a: a,
    session_b: b,
    last_message_at: "t",
  });

  it("are left out of a bot's sessions in the sidebar", () => {
    const list = [session("a"), archived("b"), session("c", "b2")];
    expect(sessionsOf(list, "b1").map((s) => s.id)).toEqual(["a"]);
    expect([isArchived(list[0]), isArchived(list[1])]).toEqual([false, true]);
  });

  it("stay in the state when archived and come back when restored", () => {
    let state = reducer(initialState, { type: "sessions_loaded", sessions: [session("a")] });
    state = reducer(state, ui({ type: "session_updated", session: archived("a") }));
    expect(state.sessions.map((s) => [s.id, isArchived(s)])).toEqual([["a", true]]);
    expect(sessionsOf(state.sessions, "b1")).toEqual([]);
    state = reducer(state, ui({ type: "session_updated", session: session("a") }));
    expect(sessionsOf(state.sessions, "b1").map((s) => s.id)).toEqual(["a"]);
  });

  it("show read-only, except in the chat that is archiving them until the archive returns", () => {
    expect(showsArchived(archived("a"), null)).toBe(true);
    expect(showsArchived(archived("a"), "a")).toBe(false);
    expect(showsArchived(archived("a"), "b")).toBe(true);
    expect(showsArchived(session("a"), null)).toBe(false);
    expect(showsArchived(null, null)).toBe(false);
  });

  it("hide a bot thread when one of its ends is archived", () => {
    const list = [session("s1"), archived("s2", "b2")];
    expect(hasArchivedEnd(thread("s1", "s2"), list)).toBe(true);
    expect(hasArchivedEnd(thread("s2", "s1"), list)).toBe(true);
    expect(hasArchivedEnd(thread("s1", null), list)).toBe(false);
    expect(hasArchivedEnd(thread("s1", "gone"), list)).toBe(false);
  });
});

describe("renamedTo", () => {
  it("is the name to send, or null when the name did not change", () => {
    expect(renamedTo("Release notes", null)).toBe("Release notes");
    expect(renamedTo("  Release notes ", "Release notes")).toBeNull();
    expect(renamedTo("", null)).toBeNull();
    expect(renamedTo("  ", "Release notes")).toBe("");
  });
});

describe("waiting for the user", () => {
  it("marks a session with a pending question, not an archived one", () => {
    expect(isWaiting(session("a", "b1", { waiting_since: "2026-10-16T10:00:00Z" }))).toBe(true);
    expect(isWaiting(session("a", "b1", { waiting_since: null }))).toBe(false);
    expect(isWaiting(session("a", "b1", { waiting_since: "t", archived_at: "t" }))).toBe(false);
  });
  it("marks a bot while any of its sessions waits", () => {
    const sessions = [session("a", "b1", { waiting_since: "t" }), session("b", "b2")];
    expect(botWaits(sessions, "b1")).toBe(true);
    expect(botWaits(sessions, "b2")).toBe(false);
  });
});
