// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type {
  BotView,
  RunGlance,
  RunView,
  SessionActivity,
  SessionView,
  ThreadSummary,
  UiEvent,
  Workflow,
  WorkflowStep,
} from "../api";
import { buildDoneTasks, buildTasks } from "../board";
import { blankDrawer, type DrawerView, type Recipient } from "../drawer";
import i18n from "../i18n";
import TaskDrawer, { HandoffList } from "./TaskDrawer";

const tauri = vi.hoisted(() => ({
  invoked: [] as [string, unknown][],
  run: null as unknown,
  listeners: [] as ((event: { payload: unknown }) => void)[],
  asked: [] as string[],
  answer: true,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (message: string) => {
    tauri.asked.push(message);
    return Promise.resolve(tauri.answer);
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    tauri.invoked.push([command, args]);
    return Promise.resolve(command === "get_workflow_run" ? tauri.run : null);
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, handler: (event: { payload: unknown }) => void) => {
    tauri.listeners.push(handler);
    return Promise.resolve(() => {});
  },
}));
vi.mock("./WorkflowMap", () => ({
  default: ({
    def,
    state,
    onOpenStep,
  }: {
    def: { steps: { id: string; name: string }[] };
    state?: { steps: Record<string, { visits: number }> };
    onOpenStep?: (id: string) => void;
  }) => (
    <div className="map">
      {def.steps.map((step) => (
        <button key={step.id} data-step={step.id} onClick={() => onOpenStep?.(step.id)}>
          {`${step.name} ×${state?.steps[step.id]?.visits ?? 0}`}
        </button>
      ))}
    </div>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = (id: string, handle: string, display_name: string): BotView => ({ project_id: "default",
  id,
  handle,
  display_name,
  color: "#4f46e5",
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status: "idle",
});
const bots = [bot("b1", "master", "Master"), bot("b2", "feature", "Feature"), bot("b3", "testing", "Testing")];

const s = (id: string, bot_id: string, extra: Partial<SessionView> = {}): SessionView => ({ project_id: "default",
  id,
  bot_id,
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

const NOW = new Date("2026-11-04T12:00:00.000Z");
const nothing = () => {};

const workflow = (id: string, name: string): Workflow => ({ project_id: "default",
  id,
  name,
  when_to_use: "",
  def: { version: 1, start: null, steps: [], start_pos: null, done_pos: null },
  created_at: "t",
  updated_at: "t",
});
const workflows = [workflow("w1", "Ticket to merge request"), workflow("w2", "Bug to fix")];

const sessions = [
  s("m", "b1", { title: "Fix the login" }),
  s("f", "b2", { opened_by: "m", created_at: "2026-11-04T09:01:00.000Z" }),
  s("t", "b3", { opened_by: "f", created_at: "2026-11-04T09:02:00.000Z", status: "working" }),
];
const [task] = buildTasks({ sessions, activity: {}, glances: {}, attentionSince: {}, paused: false, limits: [], providers: {} });

function drawer(view: DrawerView, withTask = true, activity: Record<string, SessionActivity> = {}): string {
  return renderToStaticMarkup(
    <TaskDrawer
      task={withTask ? task : null}
      drawer={view}
      bots={bots}
      sessions={sessions}
      activity={activity}
      now={NOW}
      workflows={workflows}
      onShow={nothing}
      onPick={nothing}
      onOpenFull={nothing}
      onMarkDone={nothing}
      onClose={nothing}
      onError={nothing}
    >
      <p>the chat</p>
    </TaskDrawer>,
  );
}

describe("the task drawer", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("the drawer's header has the trail, Hand-offs, Open the full chat, Mark done and Close", () => {
    const html = drawer({ root: "m", show: { kind: "session", id: "f" } });
    expect(html).toContain("Fix the login");
    for (const handle of ["@master", "@feature", "@testing"]) expect(html).toContain(handle);
    for (const label of ["Hand-offs", "Open the full chat", "Mark done", "Close"]) expect(html).toContain(label);
    expect(html).toContain("Working");
    expect(html).not.toContain("<select");
    expect(html).toContain("<p>the chat</p>");
  });

  it("marks the chat shown in the trail", () => {
    const html = drawer({ root: "m", show: { kind: "session", id: "f" } });
    const current = /<button[^>]*aria-current="true"[^>]*>([\s\S]*?)<\/button>/.exec(html);
    expect(current?.[1]).toContain("@feature");
    expect(html.split('aria-current="true"').length - 1).toBe(1);
  });

  it("marks each member of the trail with a message the user has not read", () => {
    const unread = (id: string): SessionActivity => ({
      session_id: id,
      working_on: null,
      queued: 0,
      question: null,
      last_sent: null,
      cut_off: null,
      last_reply: null,
      last_for_user_at: "2026-11-04T11:00:00.000Z",
      run_of: null,
      workflow: null,
    });
    const html = drawer({ root: "m", show: { kind: "session", id: "m" } }, true, { f: unread("f") });
    const entries = html.split('<li class="trail-entry">').slice(1);
    const marked = entries.map((entry) => entry.includes('role="img" aria-label="Unread"'));
    expect(marked).toEqual([false, true, false]);
    expect(drawer({ root: "m", show: { kind: "session", id: "m" } })).not.toContain('aria-label="Unread"');
  });

  it("marks each member of the trail that needs the user", () => {
    const needing = [
      s("m", "b1", { title: "Fix the login", status: "needs_attention" }),
      s("f", "b2", { opened_by: "m", created_at: "2026-11-04T09:01:00.000Z" }),
      s("t", "b3", { opened_by: "f", created_at: "2026-11-04T09:02:00.000Z", waiting_since: NOW.toISOString() }),
    ];
    const [both] = buildTasks({ sessions: needing, activity: {}, glances: {}, attentionSince: {}, paused: false, limits: [], providers: {} });
    const html = renderToStaticMarkup(
      <TaskDrawer
        task={both}
        drawer={{ root: "m", show: { kind: "session", id: "m" } }}
        bots={bots}
        sessions={needing}
        activity={{}}
        now={NOW}
        workflows={workflows}
        onShow={nothing}
        onPick={nothing}
        onOpenFull={nothing}
        onMarkDone={nothing}
        onClose={nothing}
        onError={nothing}
      >
        <p>the chat</p>
      </TaskDrawer>,
    );
    const entries = html.split('<li class="trail-entry">').slice(1);
    expect(entries.map((entry) => entry.includes('role="img" aria-label="Needs you"'))).toEqual([true, false, true]);
    expect(entries[0]).toContain('class="state-dot blocked"');
    expect(entries[2]).toContain('class="state-dot waits"');
    expect(drawer({ root: "m", show: { kind: "session", id: "m" } })).not.toContain('aria-label="Needs you"');
  });

  it("presses Hand-offs while they show, and offers no full chat for them", () => {
    const html = drawer({ root: "m", show: { kind: "handoffs" } });
    expect(html).toMatch(/<button[^>]*aria-pressed="true"[^>]*>Hand-offs<\/button>/);
    expect(html).not.toContain('aria-current="true"');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Open the full chat"/);
  });

  it("a blank chat's drawer offers a bot picker", () => {
    const html = drawer({ root: null, show: { kind: "newSession", botId: "b2" } }, false);
    expect(html).toContain("<select");
    for (const name of ["Master", "Feature", "Testing"]) {
      expect(html).toMatch(new RegExp(`<option[^>]*>${name}</option>`));
    }
    expect(html).toMatch(/<option value="bot:b2" selected="">Feature<\/option>/);
    expect(html).toContain("New task");
    expect(html).not.toContain("Hand-offs");
    expect(html).not.toContain("Mark done");
    expect(html).not.toContain('class="trail"');
    expect(html).toContain("Open the full chat");
    expect(html).toContain("Close");
  });
});

describe("the recipient picker", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("the picker lists bots and workflows, and picking a workflow asks for a newRun", async () => {
    const html = drawer({ root: null, show: { kind: "newSession", botId: "b2" } }, false);
    const groups = [...html.matchAll(/<optgroup label="([^"]*)">([\s\S]*?)<\/optgroup>/g)];
    expect(groups.map((g) => g[1])).toEqual(["Bots", "Workflows"]);
    expect([...groups[0][2].matchAll(/value="([^"]*)"/g)].map((m) => m[1])).toEqual(["bot:b1", "bot:b2", "bot:b3"]);
    expect([...groups[1][2].matchAll(/<option value="([^"]*)"[^>]*>([^<]*)</g)].map((m) => [m[1], m[2]])).toEqual([
      ["workflow:w1", "Ticket to merge request"],
      ["workflow:w2", "Bug to fix"],
    ]);

    const picked: Recipient[] = [];
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const view: DrawerView = { root: null, show: { kind: "newSession", botId: "b2" } };
    await act(async () =>
      root.render(
        <TaskDrawer
          task={null}
          drawer={view}
          bots={bots}
          workflows={workflows}
          sessions={sessions}
          activity={{}}
          now={NOW}
          onShow={nothing}
          onPick={(target) => picked.push(target)}
          onOpenFull={nothing}
          onMarkDone={nothing}
          onClose={nothing}
          onError={nothing}
        >
          <p>the chat</p>
        </TaskDrawer>,
      ),
    );
    const select = host.querySelector("select")!;
    await act(async () => {
      select.value = "workflow:w2";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => {
      select.value = "bot:b3";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(picked).toEqual([
      { kind: "workflow", id: "w2" },
      { kind: "bot", id: "b3" },
    ]);
    expect(blankDrawer(picked[0])).toEqual({ root: null, show: { kind: "newRun", workflowId: "w2" } });
    expect(blankDrawer(picked[1])).toEqual({ root: null, show: { kind: "newSession", botId: "b3" } });
    await act(async () => root.unmount());
    host.remove();

    const run = drawer({ root: null, show: { kind: "newRun", workflowId: "w1" } }, false);
    expect(run).toMatch(/<option value="workflow:w1" selected="">Ticket to merge request<\/option>/);
    expect(run).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Open the full chat"/);
  });
});

describe("the hand-offs list", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  const thread: ThreadSummary = {
    id: "th",
    participant_a: "b2",
    participant_b: "b3",
    session_a: "f",
    session_b: "t",
    last_message_at: "2026-11-04T10:00:00.000Z",
  };

  it("names each thread by its two ends", () => {
    const html = renderToStaticMarkup(
      <HandoffList threads={[thread]} bots={bots} sessions={sessions} onOpen={nothing} />,
    );
    expect(html).toContain("@feature");
    expect(html).toContain("@testing");
    expect(html).toContain("↔");
  });

  it("says so when no bot handed the task on", () => {
    const html = renderToStaticMarkup(<HandoffList threads={[]} bots={bots} sessions={sessions} onOpen={nothing} />);
    expect(html).toContain("No hand-offs yet");
  });
});

describe("a workflow run in the drawer", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  const glance = (extra: Partial<RunGlance> = {}): RunGlance => ({
    run_id: "r1",
    name: "Ticket to merge request",
    status: "running",
    step: "Review",
    handle: "testing",
    visit: 1,
    pause: null,
    outcomes: [],
    holder: null,
    ...extra,
  });
  const step = (id: string, name: string, bot_id: string, to: string): WorkflowStep => ({
    id,
    name,
    bot_id,
    instructions: "x",
    arrows: [{ to, outcome: null }],
    pos: null,
  });
  const runActivity = (id: string, workflow: RunGlance): SessionActivity => ({
    session_id: id,
    working_on: null,
    queued: 0,
    question: null,
    last_sent: null,
    cut_off: null,
    last_reply: null,
    last_for_user_at: null,
    run_of: null,
    workflow,
  });
  const visit = (id: string, ord: number, step_id: string, session_id: string | null) => ({
    id,
    run_id: "r1",
    ord,
    step_id,
    n: 1,
    session_id,
    request_message: "x",
    result_message: null,
    outcome: null,
    reminded: false,
    started_at: "t",
    ended_at: "t",
  });
  const run: RunView = {
    run: {
      project_id: "default",      id: "r1",
      workflow_id: "w1",
      name: "Ticket to merge request",
      def: {
        version: 1,
        start: "s1",
        steps: [step("s1", "Implement", "b2", "s2"), step("s2", "Review", "b3", "s1")],
        start_pos: null,
        done_pos: null,
      },
      request: "TASK-1",
      request_files: [],
      opener_session: null,
      root_session: "m",
      status: "running",
      pause: null,
      extra_visits: {},
      created_at: "t",
      ended_at: null,
    },
    visits: [visit("v1", 1, "s1", "m"), visit("v2", 2, "s2", "t"), visit("v3", 3, "s1", "f")],
  };
  const changed = (run_id: string): UiEvent => ({
    type: "workflow_run_changed",
    run_id,
    name: "Ticket to merge request",
    status: "running",
    pause: null,
    step: "Review",
    session_ids: ["t"],
  });

  it("shows archived workflow history without execution controls or links to live sessions", async () => {
    tauri.run = run;
    const archived = sessions.map((s) => ({ ...s, archived_at: NOW.toISOString() }));
    archived[1].archived_at = "2026-11-01T00:00:00.000Z";
    const [done] = buildDoneTasks({ sessions: archived, activity: { t: runActivity("t", glance({ status: "paused" })) } });
    const onShow = vi.fn();
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(
      <TaskDrawer
        task={done}
        drawer={{ root: done.id, show: { kind: "session", id: done.id }, archivedAt: done.root.archived_at! }}
        bots={bots} workflows={workflows} sessions={archived} activity={{}} now={NOW}
        onShow={onShow} onPick={nothing} onOpenFull={nothing} onMarkDone={nothing}
        onClose={nothing} onError={nothing}
      >
        <p>archived chat</p>
      </TaskDrawer>,
    ));
    expect(host.querySelectorAll(".map button")).toHaveLength(2);
    expect(host.querySelector(".workflow-pause")).toBeNull();
    expect(host.textContent).toContain("Restore");
    expect(host.textContent).not.toContain("Mark done");
    expect(host.textContent).not.toContain("Stop the run");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-step="s1"]')!.click());
    expect(onShow).not.toHaveBeenCalled();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-step="s2"]')!.click());
    expect(onShow).toHaveBeenCalledWith({ kind: "session", id: "t" });
    await act(async () => root.unmount());
  });

  it("maps the run, reads it again when it changes, and a step opens its latest visit's session", async () => {
    tauri.invoked = [];
    tauri.listeners = [];
    tauri.run = run;
    const [withRun] = buildTasks({
      sessions,
      activity: { t: runActivity("t", glance()) },
      glances: {},
      attentionSince: {},
      paused: false,
      limits: [],
      providers: {},
    });
    const shown: unknown[] = [];
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    await act(async () =>
      root.render(
        <TaskDrawer
          task={withRun}
          drawer={{ root: "m", show: { kind: "session", id: "m" } }}
          bots={bots}
          workflows={workflows}
          sessions={sessions}
          activity={{}}
          now={NOW}
          onShow={(show) => shown.push(show)}
          onPick={nothing}
          onOpenFull={nothing}
          onMarkDone={nothing}
          onClose={nothing}
          onError={nothing}
        >
          <p>the chat</p>
        </TaskDrawer>,
      ),
    );
    const reads = () => tauri.invoked.filter(([command]) => command === "get_workflow_run");
    expect(reads()).toEqual([["get_workflow_run", { runId: "r1" }]]);
    const boxes = [...host.querySelectorAll(".map button")].map((b) => b.textContent);
    expect(boxes).toEqual(["Implement ×2", "Review ×1"]);
    await act(async () => tauri.listeners.forEach((listener) => listener({ payload: changed("r2") })));
    expect(reads()).toHaveLength(1);
    await act(async () => tauri.listeners.forEach((listener) => listener({ payload: changed("r1") })));
    expect(reads()).toHaveLength(2);
    await act(async () => host.querySelector<HTMLElement>('[data-step="s1"]')!.click());
    expect(shown).toEqual([{ kind: "session", id: "f" }]);
    expect(host.querySelector(".workflow-pause")).toBeNull();
    await act(async () => root.unmount());
    host.remove();
  });

  it("a running run offers Stop the run, which stops it only once the user confirms", async () => {
    tauri.invoked = [];
    tauri.listeners = [];
    tauri.asked = [];
    tauri.run = run;
    const quiet = sessions.map((x): SessionView => ({ ...x, status: "idle" }));
    const [waiting] = buildTasks({
      sessions: quiet,
      activity: { t: runActivity("t", glance()) },
      glances: {},
      attentionSince: {},
      paused: false,
      limits: [],
      providers: {},
    });
    expect([waiting.column, waiting.kind]).toEqual(["running", "run_waits"]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    await act(async () =>
      root.render(
        <TaskDrawer
          task={waiting}
          drawer={{ root: "m", show: { kind: "session", id: "m" } }}
          bots={bots}
          workflows={workflows}
          sessions={quiet}
          activity={{}}
          now={NOW}
          onShow={nothing}
          onPick={nothing}
          onOpenFull={nothing}
          onMarkDone={nothing}
          onClose={nothing}
          onError={nothing}
        >
          <p>the chat</p>
        </TaskDrawer>,
      ),
    );
    expect(host.textContent).toContain("Waiting for @testing's result");
    const stop = [...host.querySelectorAll<HTMLButtonElement>(".drawer-run button")].find(
      (b) => b.textContent === "Stop the run",
    );
    expect(stop).toBeDefined();
    const stops = () => tauri.invoked.filter(([command]) => command === "stop_run");

    tauri.answer = false;
    await act(async () => stop!.click());
    expect(tauri.asked).toEqual(["Stop Ticket to merge request? A stopped run cannot be resumed."]);
    expect(stops()).toEqual([]);

    tauri.answer = true;
    await act(async () => stop!.click());
    expect(tauri.asked).toHaveLength(2);
    expect(stops()).toEqual([["stop_run", { runId: "r1" }]]);
    await act(async () => root.unmount());
    host.remove();
  });

  it("a paused run shows why at the top of the drawer", () => {
    const paused = glance({ status: "paused", pause: { kind: "bot_missing", step_id: "s2" }, handle: null });
    const [held] = buildTasks({
      sessions,
      activity: { t: runActivity("t", paused) },
      glances: {},
      attentionSince: {},
      paused: false,
      limits: [],
      providers: {},
    });
    const html = renderToStaticMarkup(
      <TaskDrawer
        task={held}
        drawer={{ root: "m", show: { kind: "session", id: "m" } }}
        bots={bots}
        workflows={workflows}
        sessions={sessions}
        activity={{}}
        now={NOW}
        onShow={nothing}
        onPick={nothing}
        onOpenFull={nothing}
        onMarkDone={nothing}
        onClose={nothing}
        onError={nothing}
      >
        <p>the chat</p>
      </TaskDrawer>,
    );
    expect(html).toContain("Step Review has no bot");
    expect(html).toContain(">Stop the run</button>");
    expect(html.indexOf("Step Review has no bot")).toBeLessThan(html.indexOf("<p>the chat</p>"));
  });
});
