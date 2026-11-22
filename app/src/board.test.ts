import { describe, expect, it } from "vitest";
import type { Glance, Message, RunGlance, SessionActivity, SessionView } from "./api";
import {
  activityTargets,
  buildTasks,
  buildDoneTasks,
  columnTasks,
  elapsed,
  glanceTargets,
  hasWorkGoingOn,
  isUnread,
  latestStep,
  noteAttention,
  taskRoots,
  trailOf,
  trailView,
  type BoardInput,
  type BoardTask,
} from "./board";

const T1 = "2026-11-04T10:00:00.000Z";
const T2 = "2026-11-04T11:00:00.000Z";
const T3 = "2026-11-04T12:00:00.000Z";

let counter = 0;
const s = (id: string, extra: Partial<SessionView> = {}): SessionView => ({ project_id: "default",
  id,
  bot_id: `bot-${id}`,
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: `2026-11-04T09:00:00.${String(100 + counter++).padStart(3, "0")}Z`,
  last_activity_at: T1,
  archived_at: null,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "idle",
  ...extra,
});
const activity = (id: string, extra: Partial<SessionActivity> = {}): SessionActivity => ({
  session_id: id,
  working_on: null,
  queued: 0,
  question: null,
  last_sent: null,
  cut_off: null,
  last_reply: null,
  last_for_user_at: null,
  run_of: null,
  workflow: null,
  ...extra,
});
const glance = (id: string, extra: Partial<Glance> = {}): Glance => ({
  session_id: id,
  step: null,
  background_work: false,
  tail: null,
  ...extra,
});
const input = (sessions: SessionView[], extra: Partial<BoardInput> = {}): BoardInput => ({
  sessions,
  activity: {},
  glances: {},
  attentionSince: {},
  paused: false,
  limits: [],
  providers: {},
  ...extra,
});

describe("tasks", () => {
  it("a chain from the root is one task", () => {
    const tasks = buildTasks(input([s("m"), s("f", { opened_by: "m" }), s("t", { opened_by: "f" })]));
    expect(tasks.map((t) => t.members.map((m) => m.id))).toEqual([["m", "f", "t"]]);
  });
  it("a child restored alone, its opener archived, is its own task", () => {
    const sessions = [s("m", { archived_at: "x" }), s("f", { opened_by: "m" })];
    expect(taskRoots(sessions).map((r) => r.id)).toEqual(["f"]);
  });
  it("an opener whose bot was deleted leaves a root", () => {
    expect(taskRoots([s("f", { opened_by: "gone" })]).map((r) => r.id)).toEqual(["f"]);
  });
  it("archiving takes a member out of its task, and the root takes the card", () => {
    const split = [s("m"), s("f", { opened_by: "m", archived_at: "x" }), s("t", { opened_by: "f", archived_at: "x" })];
    expect(buildTasks(input(split)).map((t) => t.members.map((m) => m.id))).toEqual([["m"]]);
    const done = [s("m", { archived_at: "x" }), s("f", { opened_by: "m", archived_at: "x" })];
    expect(buildTasks(input(done))).toEqual([]);
  });
});

describe("done tasks", () => {
  it("groups sessions archived together, leaving independently archived descendants accessible", () => {
    const sessions = [
      s("m", { archived_at: T2 }),
      s("f", { opened_by: "m", archived_at: T2 }),
      s("t", { opened_by: "f", archived_at: T1 }),
      s("active"),
      s("orphan", { opened_by: "missing", archived_at: T3 }),
    ];
    const tasks = buildDoneTasks(input(sessions));
    expect(tasks.map((t) => t.members.map((m) => m.id))).toEqual([["m", "f"], ["t"], ["orphan"]]);
    expect(tasks.map((t) => [t.column, t.kind, t.since])).toEqual([
      ["done", "done", T2], ["done", "done", T1], ["done", "done", T3],
    ]);
    const restored = sessions.map((s) => s.archived_at === T2 ? { ...s, archived_at: null } : s);
    expect(buildTasks(input(restored)).map((t) => t.id)).toEqual(["m", "active"]);
    expect(buildDoneTasks(input(restored)).map((t) => t.id)).toEqual(["t", "orphan"]);
  });

  it("matches restore across a separately archived or restored ancestor", () => {
    const sessions = [
      s("m", { archived_at: T2 }),
      s("f", { opened_by: "m", archived_at: T1 }),
      s("t", { opened_by: "f", archived_at: T2 }),
    ];
    expect(buildDoneTasks(input(sessions)).map((t) => t.members.map((m) => m.id))).toEqual([["m", "t"], ["f"]]);
    sessions[1].archived_at = null;
    expect(buildDoneTasks(input(sessions)).map((t) => t.members.map((m) => m.id))).toEqual([["m", "t"]]);
    expect(buildTasks(input(sessions)).map((t) => t.id)).toEqual(["f"]);
  });

  it("sorts by completion, ignores unread and conversation activity, and never folds old done tasks", () => {
    const sessions = [
      s("b", { archived_at: T2, last_activity_at: T3 }),
      s("old", { archived_at: T1, last_activity_at: T3 }),
      s("new", { archived_at: T3, last_activity_at: T1 }),
      s("a", { archived_at: T2 }),
    ];
    const tasks = buildDoneTasks(input(sessions, { activity: { old: activity("old", { last_for_user_at: T3 }) } }));
    expect(tasks.find((t) => t.id === "old")?.unread).toBe(true);
    const column = columnTasks(tasks, "done", new Date("2026-12-01T00:00:00Z"));
    expect(column.shown.map((t) => t.id)).toEqual(["new", "a", "b", "old"]);
    expect(column.older).toEqual([]);
  });

  it("stays done even while activity still reports interrupted or running work", () => {
    const [task] = buildDoneTasks(input([s("m", { archived_at: T3, status: "working" })], {
      activity: { m: activity("m", { cut_off: { message_id: "x", sender: "user", at: T2 }, queued: 2 }) },
    }));
    expect([task.column, task.kind, task.needing, task.owes]).toEqual(["done", "done", [], null]);
  });
});

describe("columns", () => {
  it("a question deep in the task holds the card", () => {
    const sessions = [
      s("m"),
      s("f", { opened_by: "m" }),
      s("t", { opened_by: "f", waiting_since: T1, waiting_question_id: "q" }),
    ];
    const [task] = buildTasks(input(sessions));
    expect([task.column, task.kind, task.holder.id, task.since]).toEqual(["needs_you", "question", "t", T1]);
    expect(task.needing).toEqual([{ id: "t", kind: "question" }]);
  });
  it("needs you wins over work elsewhere in the task", () => {
    const sessions = [
      s("m", { status: "working" }),
      s("t", { opened_by: "m", waiting_since: T1, waiting_question_id: "q" }),
    ];
    const [task] = buildTasks(input(sessions));
    expect([task.column, task.kind, task.holder.id]).toEqual(["needs_you", "question", "t"]);
  });
  it("a dialog counts from when it was seen", () => {
    const [task] = buildTasks(input([s("m", { status: "needs_attention" })], { attentionSince: { m: T2 } }));
    expect([task.column, task.kind, task.since]).toEqual(["needs_you", "dialog", T2]);
    const [bare] = buildTasks(input([s("m", { status: "needs_attention" })]));
    expect(bare.since).toBeNull();
  });
  it("a cut-off message puts the task in needs you", () => {
    const sessions = [s("m"), s("f", { opened_by: "m" })];
    const cut = { message_id: "x", sender: "m", at: T2 };
    const [task] = buildTasks(input(sessions, { activity: { f: activity("f", { cut_off: cut }) } }));
    expect([task.column, task.kind, task.holder.id, task.since]).toEqual(["needs_you", "cut_off", "f", T2]);
  });
  it("the longest-waiting needing member holds a needs-you task", () => {
    const sessions = [
      s("m"),
      s("a", { opened_by: "m", waiting_since: T2, waiting_question_id: "q1" }),
      s("b", { opened_by: "m", waiting_since: T1, waiting_question_id: "q2" }),
    ];
    const [task] = buildTasks(input(sessions));
    expect(task.holder.id).toBe("b");
    expect(task.needing.map((n) => n.id)).toEqual(["a", "b"]);
  });
  it("every member that needs the user counts, whatever it needs", () => {
    const sessions = [
      s("m", { status: "needs_attention" }),
      s("a", { opened_by: "m", waiting_since: T2, waiting_question_id: "q1" }),
      s("b", { opened_by: "m" }),
      s("c", { opened_by: "m", status: "working" }),
    ];
    const cut = { message_id: "x", sender: "m", at: T1 };
    const [task] = buildTasks(input(sessions, { activity: { b: activity("b", { cut_off: cut }) } }));
    expect(task.needing).toEqual([
      { id: "m", kind: "dialog" },
      { id: "a", kind: "question" },
      { id: "b", kind: "cut_off" },
    ]);
    expect(buildTasks(input([s("m")]))[0].needing).toEqual([]);
  });
  it("the newest working member holds a running task", () => {
    const sessions = [
      s("m"),
      s("f", { opened_by: "m", status: "working" }),
      s("t", { opened_by: "f", status: "working" }),
    ];
    const acts = { t: activity("t", { working_on: { text: "x", at: T3 } }) };
    const [task] = buildTasks(input(sessions, { activity: acts }));
    expect([task.column, task.kind, task.holder.id, task.since]).toEqual(["running", "working", "t", T3]);
  });
  it("a starting member is running", () => {
    const [task] = buildTasks(input([s("m", { status: "starting" })]));
    expect([task.column, task.kind, task.since]).toEqual(["running", "starting", null]);
  });
  it("a queued message is running, and paused while paused", () => {
    const acts = { m: activity("m", { queued: 1 }) };
    expect(buildTasks(input([s("m")], { activity: acts }))[0].kind).toBe("queued");
    const [paused] = buildTasks(input([s("m")], { activity: acts, paused: true }));
    expect([paused.column, paused.kind]).toEqual(["running", "paused"]);
  });
  it("a limited session is running, with its provider's limit", () => {
    const limit = { provider: "claude" as const, resets_at: "2026-11-05T15:00:00.000Z" };
    const [task] = buildTasks(input([s("m", { status: "limited" })], { limits: [limit], providers: { "bot-m": "claude" } }));
    expect([task.column, task.kind, task.limit]).toEqual(["running", "limited", limit]);
  });
  it("a limited session in the grace has no limit, and is still limited", () => {
    const [task] = buildTasks(input([s("m", { status: "limited" })], { providers: { "bot-m": "claude" } }));
    expect([task.kind, task.limit]).toEqual(["limited", null]);
  });
  it("queued work whose provider is held is limited, even paused; with the hold gone it is queued", () => {
    const acts = { m: activity("m", { queued: 1 }) };
    const limits = [{ provider: "claude" as const, resets_at: null }];
    const providers = { "bot-m": "claude" as const };
    const [limited] = buildTasks(input([s("m")], { activity: acts, paused: true, limits, providers }));
    expect([limited.kind, limited.limit]).toEqual(["limited", limits[0]]);
    const [free] = buildTasks(input([s("m")], { activity: acts, providers }));
    expect([free.kind, free.limit]).toEqual(["queued", null]);
    const [other] = buildTasks(input([s("m")], { activity: acts, limits, providers: { "bot-m": "grok" } }));
    expect(other.kind).toBe("queued");
  });
  it("an idle session with background work is running", () => {
    const [task] = buildTasks(input([s("m")], { glances: { m: glance("m", { background_work: true }) } }));
    expect([task.column, task.kind]).toEqual(["running", "background"]);
  });
  it("a quiet task is finished and names a member that owes its result", () => {
    const sessions = [
      s("m", { last_activity_at: T1 }),
      s("f", { opened_by: "m", open_request_id: "r", last_activity_at: T2 }),
    ];
    const [task] = buildTasks(input(sessions));
    expect([task.column, task.kind, task.holder.id, task.since, task.quietSince]).toEqual([
      "finished",
      "finished",
      "m",
      T2,
      T2,
    ]);
    expect(task.owes?.id).toBe("f");
    expect(buildTasks(input([s("m")]))[0].owes).toBeNull();
  });
  it("owes is only named for a finished task", () => {
    const sessions = [s("m", { status: "working" }), s("f", { opened_by: "m", open_request_id: "r" })];
    expect(buildTasks(input(sessions))[0].owes).toBeNull();
  });
});

describe("work going on", () => {
  const going = (extra: Partial<SessionView>, seen: Partial<BoardInput> = {}) => {
    const all = input([s("m"), s("f", { opened_by: "m", ...extra })], seen);
    return hasWorkGoingOn(buildTasks(all)[0], all.activity, all.glances);
  };
  it("a member starting, working or in a dialog is work going on", () => {
    for (const status of ["starting", "working", "needs_attention"] as const) expect(going({ status })).toBe(true);
  });
  it("a question with a working member elsewhere is work going on", () => {
    const sessions = [s("m", { status: "working" }), s("f", { opened_by: "m", waiting_since: T1 })];
    const [task] = buildTasks(input(sessions));
    expect(task.column).toBe("needs_you");
    expect(hasWorkGoingOn(task, {}, {})).toBe(true);
  });
  it("a message in flight, one queued or background work is work going on", () => {
    expect(going({}, { activity: { f: activity("f", { working_on: { text: "x", at: T1 } }) } })).toBe(true);
    expect(going({}, { activity: { f: activity("f", { queued: 1 }) } })).toBe(true);
    expect(going({}, { glances: { f: glance("f", { background_work: true }) } })).toBe(true);
  });
  it("a quiet task, or one only waiting on the user, has none", () => {
    expect(going({})).toBe(false);
    expect(going({ status: "offline" })).toBe(false);
    expect(going({ waiting_since: T1, waiting_question_id: "q" })).toBe(false);
    const cut = { message_id: "x", sender: "user", at: T1 };
    expect(going({}, { activity: { f: activity("f", { cut_off: cut }) }, glances: { f: glance("f") } })).toBe(false);
  });
});

describe("unread and recurring", () => {
  it("unread means a message for the user after the last read", () => {
    expect(isUnread([s("m", { seen_at: T1 })], { m: activity("m", { last_for_user_at: T2 }) })).toBe(true);
    expect(isUnread([s("m", { seen_at: T2 })], { m: activity("m", { last_for_user_at: T1 }) })).toBe(false);
    expect(isUnread([s("m", { seen_at: null })], { m: activity("m", { last_for_user_at: T1 }) })).toBe(true);
    expect(isUnread([s("m", { seen_at: null })], {})).toBe(false);
  });
  it("a task is unread through any member", () => {
    const sessions = [s("m", { seen_at: T3 }), s("f", { opened_by: "m", seen_at: T1 })];
    const acts = { f: activity("f", { last_for_user_at: T2 }) };
    expect(buildTasks(input(sessions, { activity: acts }))[0].unread).toBe(true);
  });
  it("a recurring run carries its task's name", () => {
    const [task] = buildTasks(input([s("m")], { activity: { m: activity("m", { run_of: "Wiki" }) } }));
    expect(task.recurring).toBe("Wiki");
    expect(buildTasks(input([s("m")]))[0].recurring).toBeNull();
  });
});

const run = (extra: Partial<RunGlance> = {}): RunGlance => ({
  run_id: "r1",
  name: "Ticket to merge request",
  status: "running",
  step: "Review",
  handle: "review",
  visit: 2,
  pause: null,
  outcomes: [],
  holder: null,
  ...extra,
});

describe("workflow runs", () => {
  it("a paused run puts its task in Needs you, held by the session it names", () => {
    const sessions = [s("m"), s("f", { opened_by: "m" }), s("v", { opened_by: "m", status: "working" })];
    const paused = run({ status: "paused", pause: { kind: "no_result", step_id: "s2" }, holder: "f" });
    const acts = { m: activity("m", { workflow: paused }), f: activity("f", { workflow: paused }) };
    const [task] = buildTasks(input(sessions, { activity: acts }));
    expect([task.column, task.kind, task.holder.id]).toEqual(["needs_you", "workflow", "f"]);
    expect(task.workflow).toEqual(paused);
    for (const holder of ["elsewhere", null]) {
      const away = run({ status: "paused", pause: { kind: "bot_missing", step_id: "s2" }, holder });
      const acts = { g: activity("g", { workflow: away }) };
      const [lone] = buildTasks(input([s("r"), s("g", { opened_by: "r" })], { activity: acts }));
      expect([lone.column, lone.kind, lone.holder.id]).toEqual(["needs_you", "workflow", "r"]);
    }
    const asking = [s("m"), s("f", { opened_by: "m", waiting_since: T1, waiting_question_id: "q" })];
    const [question] = buildTasks(input(asking, { activity: { m: activity("m", { workflow: paused }) } }));
    expect([question.column, question.kind, question.holder.id]).toEqual(["needs_you", "question", "f"]);
    expect(question.workflow).toEqual(paused);
  });
  it("a running run with no member at work stays in Running, waiting for its step's result", () => {
    const sessions = [s("m"), s("f", { opened_by: "m" })];
    const [task] = buildTasks(input(sessions, { activity: { f: activity("f", { workflow: run() }) } }));
    expect([task.column, task.kind, task.holder.id]).toEqual(["running", "run_waits", "m"]);
    expect(task.owes).toBeNull();
    for (const status of ["finished", "stopped"] as const) {
      const over = { f: activity("f", { workflow: run({ status, step: null }) }) };
      const [done] = buildTasks(input(sessions, { activity: over }));
      expect([done.column, done.kind]).toEqual(["finished", "finished"]);
    }
  });
  it("a running run's glance reaches the task", () => {
    const running = run();
    const sessions = [s("m", { status: "working" }), s("f", { opened_by: "m" })];
    const [task] = buildTasks(input(sessions, { activity: { f: activity("f", { workflow: running }) } }));
    expect([task.column, task.kind, task.holder.id]).toEqual(["running", "working", "m"]);
    expect(task.workflow).toEqual(running);
    expect(buildTasks(input([s("q")]))[0].workflow).toBeNull();
    const paused = run({ run_id: "r2", status: "paused", pause: { kind: "visit_cap", step_id: "s1" } });
    const done = run({ run_id: "r3", status: "finished" });
    const stopped = run({ run_id: "r4", status: "stopped" });
    const three = [s("a"), s("b", { opened_by: "a" }), s("c", { opened_by: "a" })];
    const glances = (a: RunGlance, b: RunGlance, c: RunGlance) => ({
      a: activity("a", { workflow: a }),
      b: activity("b", { workflow: b }),
      c: activity("c", { workflow: c }),
    });
    expect(buildTasks(input(three, { activity: glances(paused, running, done) }))[0].workflow).toEqual(paused);
    expect(buildTasks(input(three, { activity: glances(done, running, stopped) }))[0].workflow).toEqual(running);
    expect(buildTasks(input(three, { activity: glances(done, done, stopped) }))[0].workflow).toEqual(stopped);
  });
  it("workflow_run_changed re-reads its sessions", () => {
    expect(
      activityTargets({
        type: "workflow_run_changed",
        run_id: "r1",
        name: "Ticket to merge request",
        status: "paused",
        pause: { kind: "visit_cap", step_id: "s1" },
        step: "Review",
        session_ids: ["a", "b"],
      }),
    ).toEqual(["a", "b"]);
    expect(activityTargets({ type: "workflows_changed" })).toEqual([]);
  });
});

const msg = (extra: Partial<Message>): Message => ({ id: "x", ...extra }) as Message;
const task = (id: string, extra: Partial<BoardTask> = {}): BoardTask => {
  const root = s(id);
  return {
    id,
    root,
    members: [root],
    holder: root,
    column: "finished",
    kind: "finished",
    since: null,
    quietSince: T1,
    unread: false,
    needing: [],
    owes: null,
    recurring: null,
    limit: null,
    workflow: null,
    ...extra,
  };
};
const ids = (ts: BoardTask[]) => ts.map((x) => x.id);

describe("order, fold and trail", () => {
  const now = new Date("2026-11-04T13:00:00.000Z");
  it("needs you lists the longest wait first", () => {
    const a = task("a", { column: "needs_you", since: T2 });
    const b = task("b", { column: "needs_you", since: T1 });
    const c = task("c", { column: "needs_you", since: null });
    expect(ids(columnTasks([c, a, b], "needs_you", now).shown)).toEqual(["b", "a", "c"]);
  });
  it("running lists the longest running first, ties by id", () => {
    const a = task("a", { column: "running", since: T1 });
    const b = task("b", { column: "running", since: T1 });
    const c = task("c", { column: "finished", since: T1 });
    expect(ids(columnTasks([b, c, a], "running", now).shown)).toEqual(["a", "b"]);
  });
  it("finished lists unread before read, the newest first", () => {
    const readA = task("readA", { quietSince: T1 });
    const unreadB = task("unreadB", { quietSince: T3, unread: true });
    const readC = task("readC", { quietSince: T2 });
    const unreadD = task("unreadD", {
      quietSince: "2026-11-04T12:30:00.000Z",
      unread: true,
    });
    expect(ids(columnTasks([readA, unreadB, readC, unreadD], "finished", now).shown)).toEqual([
      "unreadD",
      "unreadB",
      "readC",
      "readA",
    ]);
  });
  it("old read tasks fold and unread ones never do", () => {
    const late = new Date("2026-11-20T00:00:00.000Z");
    const readOld = task("readOld", { quietSince: T1 });
    const unreadOld = task("unreadOld", { quietSince: T1, unread: true });
    const readNew = task("readNew", { quietSince: "2026-11-18T00:00:00.000Z" });
    const { shown, older } = columnTasks([readOld, unreadOld, readNew], "finished", late);
    expect(ids(older)).toEqual(["readOld"]);
    expect(ids(shown)).toEqual(["unreadOld", "readNew"]);
  });
  it("the other columns have no fold", () => {
    const late = new Date("2027-01-01T00:00:00.000Z");
    expect(columnTasks([task("a", { column: "running", since: T1 })], "running", late).older).toEqual([]);
  });
  it("the trail numbers a bot's second session", () => {
    const m = s("m", { bot_id: "b1" });
    const f = s("f", { bot_id: "b2", opened_by: "m" });
    const t = s("t", { bot_id: "b2", opened_by: "f" });
    expect(trailOf(task("m", { root: m, members: [m, f, t] })).map((e) => e.n)).toEqual([null, 1, 2]);
  });
  it("the trail keeps the holder in view past four", () => {
    const six = ["e1", "e2", "e3", "e4", "e5", "e6"].map((id) => ({
      session: s(id),
      n: null,
    }));
    const view = trailView(six, "e6");
    expect(view.shown.map((e) => e.session.id)).toEqual(["e1", "e2", "e3", "e6"]);
    expect(view.more).toBe(2);
    const first = trailView(six, "e2");
    expect(first.shown.map((e) => e.session.id)).toEqual(["e1", "e2", "e3", "e4"]);
    expect(first.more).toBe(2);
  });
});

describe("step, refresh and attention", () => {
  const m = s("m", { bot_id: "b-master" });
  const f = s("f", { bot_id: "b-feature", opened_by: "m" });
  const t = s("t", { bot_id: "b-testing", opened_by: "f" });
  const full = task("m", { root: m, members: [m, f, t], holder: t });
  const acts = {
    m: activity("m", {
      last_sent: { recipient: "b-feature", text: "", at: T1 },
    }),
    f: activity("f", {
      last_sent: {
        recipient: "b-testing",
        text: "Write request specs",
        at: T2,
      },
    }),
  };
  it("the latest step is the holder's screen line", () => {
    expect(latestStep(full, acts, { t: glance("t", { step: "Bash(rspec)" }) })).toEqual({
      kind: "screen",
      text: "Bash(rspec)",
    });
  });
  it("the latest step falls back to the latest message sent", () => {
    expect(latestStep(full, acts, {})).toEqual({
      kind: "sent",
      from: f,
      recipient: "b-testing",
      text: "Write request specs",
    });
    expect(latestStep(task("lone"), {}, {})).toBeNull();
  });
  it("an event names the sessions whose activity to read again", () => {
    expect(
      activityTargets({
        type: "message_added",
        message: msg({ sender_session: "a", recipient_session: "b" }),
      }),
    ).toEqual(["a", "b"]);
    expect(
      activityTargets({
        type: "message_updated",
        message: msg({ sender_session: null, recipient_session: "b" }),
      }),
    ).toEqual(["b"]);
    expect(
      activityTargets({
        type: "session_status",
        session_id: "q",
        bot_id: "x",
        status: "idle",
      }),
    ).toEqual(["q"]);
    expect(activityTargets({ type: "session_added", session: s("n") })).toEqual(["n"]);
    expect(activityTargets({ type: "session_updated", session: s("u") })).toEqual(["u"]);
    expect(activityTargets({ type: "paused", paused: true })).toEqual([]);
  });
  it("glances go to sessions with a running CLI", () => {
    const list = [
      s("w", { status: "working" }),
      s("i", { status: "idle" }),
      s("n", { status: "needs_attention" }),
      s("o", { status: "offline" }),
      s("a", { status: "working", archived_at: "x" }),
    ];
    expect(glanceTargets(list)).toEqual(["w", "i", "n"]);
  });
  it("a dialog keeps the time it was first seen", () => {
    const first = noteAttention({}, [s("a", { status: "needs_attention" })], T1);
    expect(first).toEqual({ a: T1 });
    expect(noteAttention(first, [s("a", { status: "needs_attention" })], T2)).toBe(first);
    expect(noteAttention(first, [s("a", { status: "idle" })], T2)).toEqual({});
  });
  it("elapsed time is in the largest whole unit", () => {
    const now = new Date("2026-11-04T12:00:00.000Z");
    const ago = (ms: number) => new Date(now.getTime() - ms).toISOString();
    expect(elapsed(ago(30_000), now)).toEqual({ unit: "now", n: 0 });
    expect(elapsed(ago(12 * 60_000), now)).toEqual({ unit: "min", n: 12 });
    expect(elapsed(ago(3 * 3_600_000), now)).toEqual({ unit: "h", n: 3 });
    expect(elapsed(ago(2 * 86_400_000), now)).toEqual({ unit: "d", n: 2 });
  });
});
