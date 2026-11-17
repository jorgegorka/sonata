import { describe, expect, it } from "vitest";
import type { SessionView, ThreadSummary } from "./api";
import { buildTasks } from "./board";
import {
  blankDrawer,
  chatShown,
  drawerAfter,
  followsInDrawer,
  followsRunStart,
  fullChatOf,
  handoffsOf,
  openTask,
  threadShown,
  type DrawerView,
} from "./drawer";

const s = (id: string, extra: Partial<SessionView> = {}): SessionView => ({
  id,
  bot_id: "b1",
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "2026-11-04T09:00:00.000Z",
  last_activity_at: "2026-11-04T10:00:00.000Z",
  archived_at: null,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "idle",
  ...extra,
});

const thread = (id: string, session_a: string | null, session_b: string | null): ThreadSummary => ({
  id,
  participant_a: "b1",
  participant_b: "b2",
  session_a,
  session_b,
  last_message_at: "2026-11-04T10:00:00.000Z",
});

const view = (root: string, shown: string): DrawerView => ({ root, show: { kind: "session", id: shown } });

function taskOf(sessions: SessionView[]) {
  const [task] = buildTasks({ sessions, activity: {}, glances: {}, attentionSince: {}, paused: false, limits: [], providers: {} });
  return task;
}

const chain = (t: Partial<SessionView> = {}) => [
  s("m"),
  s("f", { bot_id: "b2", opened_by: "m", created_at: "2026-11-04T09:01:00.000Z" }),
  s("t", { bot_id: "b3", opened_by: "f", created_at: "2026-11-04T09:02:00.000Z", ...t }),
];

describe("openTask", () => {
  it("a task opens on the chat that needs you, else the root's", () => {
    const waiting = taskOf(chain({ waiting_question_id: "q1", waiting_since: "2026-11-04T11:00:00.000Z" }));
    expect(waiting.holder.id).toBe("t");
    expect(openTask(waiting)).toEqual({ root: "m", show: { kind: "session", id: "t" } });

    const blocked = taskOf(chain({ status: "needs_attention" }));
    expect(openTask(blocked)).toEqual({ root: "m", show: { kind: "session", id: "t" } });

    const finished = taskOf(chain());
    expect(finished.column).toBe("finished");
    expect(openTask(finished)).toEqual({ root: "m", show: { kind: "session", id: "m" } });
  });

  it("a running task opens on the root's chat, not on the bot that works", () => {
    const running = taskOf(chain({ status: "working" }));
    expect(running.holder.id).toBe("t");
    expect(openTask(running)).toEqual({ root: "m", show: { kind: "session", id: "m" } });
  });
});

describe("drawerAfter", () => {
  it("the drawer closes when its task is marked done", () => {
    expect(drawerAfter(view("m", "f"), [s("m", { archived_at: "x" })], [])).toBeNull();
  });

  it("the drawer closes when its root is removed", () => {
    expect(drawerAfter(view("m", "f"), [s("f", { opened_by: null })], [])).toBeNull();
  });

  it("the drawer goes back to the root when the session shown leaves", () => {
    const archived = chain();
    archived[1] = { ...archived[1], archived_at: "x" };
    expect(drawerAfter(view("m", "f"), archived, [])).toEqual(view("m", "m"));
    expect(drawerAfter(view("m", "f"), [s("m")], [])).toEqual(view("m", "m"));
  });

  it("the drawer goes back to the root when the thread shown is gone or archived", () => {
    const sessions = chain();
    const onThread: DrawerView = { root: "m", show: { kind: "thread", id: "th" } };
    expect(drawerAfter(onThread, sessions, [])).toEqual(view("m", "m"));
    const archivedEnd = [...sessions, s("x", { archived_at: "x" })];
    expect(drawerAfter(onThread, archivedEnd, [thread("th", "f", "x")])).toEqual(view("m", "m"));
  });

  it("the drawer stays as it is while what it shows is there", () => {
    const sessions = chain();
    const threads = [thread("th", "m", "f")];
    for (const drawer of [
      view("m", "f"),
      { root: "m", show: { kind: "thread", id: "th" } },
      { root: "m", show: { kind: "handoffs" } },
      { root: null, show: { kind: "newSession", botId: "b1" } },
    ] satisfies DrawerView[]) {
      expect(drawerAfter(drawer, sessions, threads)).toBe(drawer);
    }
  });
});

describe("followsInDrawer", () => {
  it("a first send follows only the drawer it came from", () => {
    const created = s("n", { bot_id: "b1" });
    expect(followsInDrawer({ root: null, show: { kind: "newSession", botId: "b1" } }, created)).toBe(true);
    expect(followsInDrawer({ root: null, show: { kind: "newSession", botId: "b2" } }, created)).toBe(false);
    expect(followsInDrawer(null, created)).toBe(false);
  });

  it("a drawer on a task's chat is not taken over by a first send", () => {
    const created = s("n", { bot_id: "b1" });
    expect(followsInDrawer(view("m", "m"), created)).toBe(false);
    expect(followsInDrawer({ root: "m", show: { kind: "handoffs" } }, created)).toBe(false);
  });
});

describe("handoffsOf", () => {
  it("hand-offs are the threads with an end among the task's members", () => {
    const members = chain();
    const between = thread("between", "m", "f");
    const outside = thread("outside", "t", "other");
    const deletedBot = thread("deletedBot", null, "f");
    const elsewhere = thread("elsewhere", "x", "y");
    const raw = thread("raw", null, null);
    expect(handoffsOf(members, [between, outside, deletedBot, elsewhere, raw])).toEqual([
      between,
      outside,
      deletedBot,
    ]);
  });
});

describe("chatShown", () => {
  it("the chat shown is the full page's, else the drawer's", () => {
    expect(chatShown({ kind: "session", id: "a" }, view("m", "f"))).toBe("a");
    expect(chatShown({ kind: "board" }, view("m", "f"))).toBe("f");
    expect(chatShown({ kind: "board" }, { root: "m", show: { kind: "handoffs" } })).toBeNull();
    expect(chatShown({ kind: "archive" }, null)).toBeNull();
  });

  it("is none for a blank chat, a bot thread, or a drawer off the board", () => {
    expect(chatShown({ kind: "board" }, { root: null, show: { kind: "newSession", botId: "b1" } })).toBeNull();
    expect(chatShown({ kind: "board" }, { root: "m", show: { kind: "thread", id: "th" } })).toBeNull();
    expect(chatShown({ kind: "thread", id: "th" }, null)).toBeNull();
    expect(chatShown({ kind: "archive" }, view("m", "f"))).toBeNull();
    expect(chatShown(null, null)).toBeNull();
  });
});

describe("threadShown", () => {
  it("is the full page's thread, else the drawer's", () => {
    expect(threadShown({ kind: "thread", id: "a" }, null)).toBe("a");
    expect(threadShown({ kind: "board" }, { root: "m", show: { kind: "thread", id: "th" } })).toBe("th");
    expect(threadShown({ kind: "board" }, view("m", "f"))).toBeNull();
    expect(threadShown({ kind: "archive" }, { root: "m", show: { kind: "thread", id: "th" } })).toBeNull();
  });
});

describe("fullChatOf", () => {
  it("is the chat the drawer shows, and none for its Hand-offs", () => {
    expect(fullChatOf(view("m", "f"))).toEqual({ kind: "session", id: "f" });
    expect(fullChatOf({ root: null, show: { kind: "newSession", botId: "b1" } })).toEqual({
      kind: "newSession",
      botId: "b1",
    });
    expect(fullChatOf({ root: "m", show: { kind: "thread", id: "th" } })).toEqual({ kind: "thread", id: "th" });
    expect(fullChatOf({ root: "m", show: { kind: "handoffs" } })).toBeNull();
  });

  it("is none for a workflow's run to start, which has no chat yet", () => {
    expect(fullChatOf({ root: null, show: { kind: "newRun", workflowId: "w1" } })).toBeNull();
  });
});

describe("a blank drawer", () => {
  it("is a bot's blank chat, or a workflow's run to start", () => {
    expect(blankDrawer({ kind: "bot", id: "b1" })).toEqual({ root: null, show: { kind: "newSession", botId: "b1" } });
    expect(blankDrawer({ kind: "workflow", id: "w1" })).toEqual({
      root: null,
      show: { kind: "newRun", workflowId: "w1" },
    });
  });

  it("a started run follows only the drawer it came from", () => {
    const run = (workflowId: string): DrawerView => ({ root: null, show: { kind: "newRun", workflowId } });
    expect(followsRunStart(run("w1"), "w1")).toBe(true);
    expect(followsRunStart(run("w2"), "w1")).toBe(false);
    expect(followsRunStart({ root: null, show: { kind: "newSession", botId: "b1" } }, "w1")).toBe(false);
    expect(followsRunStart(view("m", "m"), "w1")).toBe(false);
    expect(followsRunStart(null, "w1")).toBe(false);
  });
});
