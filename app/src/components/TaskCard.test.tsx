// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { BotView, Glance, RunGlance, SessionActivity, SessionView, UsageLimit } from "../api";
import { buildTasks } from "../board";
import i18n from "../i18n";
import styles from "../styles.css?raw";
import { formatReset } from "../limits";
import TaskCard, { answerTarget } from "./TaskCard";

const tauri = vi.hoisted(() => ({
  invoked: [] as [string, unknown][],
  answer: null as null | (() => Promise<unknown>),
  asked: [] as string[],
  confirm: true,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (message: string) => {
    tauri.asked.push(message);
    return Promise.resolve(tauri.confirm);
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    tauri.invoked.push([command, args]);
    return tauri.answer ? tauri.answer() : Promise.resolve(null);
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = (id: string, handle: string, display_name: string, color: string): BotView => ({ project_id: "default",
  id,
  handle,
  display_name,
  color,
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status: "idle",
});
const bots = [
  bot("b1", "master", "Master", "#4f46e5"),
  bot("b2", "feature", "Feature", "#16a34a"),
  bot("b3", "testing", "Testing", "#0d9488"),
];

const NOW = new Date("2026-11-04T12:00:00.000Z");
const minutesAgo = (n: number) => new Date(NOW.getTime() - n * 60_000).toISOString();

const s = (id: string, bot_id: string, extra: Partial<SessionView> = {}): SessionView => ({ project_id: "default",
  id,
  bot_id,
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: `2026-11-04T09:00:00.00${id.length}Z`,
  last_activity_at: minutesAgo(30),
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

const chain = (m: Partial<SessionView> = {}, f: Partial<SessionView> = {}, t: Partial<SessionView> = {}) => [
  s("m", "b1", m),
  s("f", "b2", { opened_by: "m", ...f }),
  s("t", "b3", { opened_by: "f", ...t }),
];

interface Seen {
  activity?: SessionActivity[];
  glances?: Glance[];
  limits?: UsageLimit[];
  paused?: boolean;
}

function taskOf(sessions: SessionView[], seen: Seen = {}) {
  const activityById = Object.fromEntries((seen.activity ?? []).map((a) => [a.session_id, a]));
  const glancesById = Object.fromEntries((seen.glances ?? []).map((g) => [g.session_id, g]));
  const tasks = buildTasks({
    sessions,
    activity: activityById,
    glances: glancesById,
    attentionSince: {},
    paused: seen.paused ?? false,
    limits: seen.limits ?? [],
    providers: Object.fromEntries(bots.map((b) => [b.id, b.provider])),
  });
  expect(tasks).toHaveLength(1);
  return { task: tasks[0], activity: activityById, glances: glancesById };
}

function card(sessions: SessionView[], seen: Seen = {}): string {
  const { task, activity, glances } = taskOf(sessions, seen);
  const nothing = () => {};
  const html = renderToStaticMarkup(
    <TaskCard
      task={task}
      bots={bots}
      sessions={sessions}
      activity={activity}
      glances={glances}
      now={NOW}
      onOpen={nothing}
      onMarkDone={nothing}
      onError={nothing}
    />,
  );
  return html.replace(/&#x27;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, "&");
}

describe("a task card", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("a question card shows the question and an answer box", () => {
    const question = "Should the export include archived items,\nor only the active ones?";
    const html = card(chain({}, {}, { waiting_since: minutesAgo(18), waiting_question_id: "q1" }), {
      activity: [activity("t", { question })],
    });
    expect(html).toContain(question);
    expect(html).toContain("Asks you · 18 min");
    expect(html).toContain("<textarea");
    expect(html).toContain('placeholder="Answer @testing…"');
    expect(html).toContain(">Send</button>");
    expect(html).toContain(">Session 1</button>");
    expect(html.split('aria-current="true"')).toHaveLength(2);
    const holder = html.slice(html.indexOf('aria-current="true"'));
    expect(holder.slice(0, holder.indexOf("</li>"))).toContain("@testing");
    expect(html).not.toContain("more");
  });

  it("a question card cuts a long question to three lines, as a finished card cuts its reply", () => {
    const clamp = (selector: string) => new RegExp(`${selector} \\{([^}]*)\\}`).exec(styles)?.[1] ?? "";
    for (const rule of [clamp("\\.task-question"), clamp("\\.task-reply")]) {
      expect(rule).toContain("-webkit-line-clamp: 3;");
      expect(rule).toContain("overflow: hidden;");
      expect(rule).not.toContain("pre-wrap");
    }
  });

  it("a card with two questions shows the oldest and how many more", () => {
    const html = card(
      chain(
        {},
        { waiting_since: minutesAgo(5), waiting_question_id: "q2" },
        { waiting_since: minutesAgo(18), waiting_question_id: "q1" },
      ),
      { activity: [activity("f", { question: "The newer one?" }), activity("t", { question: "The older one?" })] },
    );
    expect(html).toContain("The older one?");
    expect(html).not.toContain("The newer one?");
    expect(html).toContain("+1 more");
  });

  it("a cut-off card says how many more members need the user", () => {
    const html = card(chain({}, { waiting_since: minutesAgo(1), waiting_question_id: "q1" }), {
      activity: [
        activity("f", { question: "Which one?" }),
        activity("t", { cut_off: { message_id: "msg2", sender: "b2", at: minutesAgo(3) } }),
      ],
    });
    expect(html).toContain("The message to @testing was cut off");
    expect(html).toContain("+1 more");
    const alone = activity("f", { cut_off: { message_id: "m", sender: "user", at: minutesAgo(3) } });
    expect(card([s("f", "b2")], { activity: [alone] })).not.toContain("more");
  });

  it("the answer box sends to the session that asked", () => {
    const { task } = taskOf(chain({}, {}, { waiting_since: minutesAgo(18), waiting_question_id: "q1" }), {
      activity: [activity("t", { question: "Which one?" })],
    });
    expect(task.holder.id).toBe("t");
    expect(answerTarget(task)).toEqual({ botId: "b3", sessionId: "t" });
  });

  it("a dialog card shows the dialog's last lines and its three actions", () => {
    const tail = "Allow Bash(psql …)?\n❯ 1. Yes\n  2. No";
    const html = card(chain({}, {}, { status: "needs_attention" }), { glances: [glance("t", { tail })] });
    expect(html).toContain(`<pre class="task-tail">${tail}</pre>`);
    expect(html).toContain(">Continue</button>");
    expect(html).toContain(">Cancel</button>");
    expect(html).toContain(">Restart session</button>");
    expect(html).not.toContain("<textarea");
  });

  it("a dialog card wraps the terminal's lines instead of cutting them at its edge", () => {
    const rule = /\.task-tail \{([^}]*)\}/.exec(styles)?.[1] ?? "";
    expect(rule).toContain("white-space: pre-wrap;");
    expect(rule).toContain("overflow-wrap: anywhere;");
  });

  it("a cut-off card names what stopped and offers Retry", () => {
    const own = card([s("f", "b2")], {
      activity: [activity("f", { cut_off: { message_id: "msg1", sender: "user", at: minutesAgo(3) } })],
    });
    expect(own).toContain("@feature's turn was cut off");
    expect(own).toContain("Blocked · 3 min");
    expect(own).toContain(">Retry</button>");

    const sent = card(chain(), {
      activity: [activity("t", { cut_off: { message_id: "msg2", sender: "b2", at: minutesAgo(3) } })],
    });
    expect(sent).toContain("The message to @testing was cut off");
    expect(sent).not.toContain("turn was cut off");
    expect(sent).toContain(">Retry</button>");
  });

  it("a running card shows the request and the latest step", () => {
    const request = { text: "Add a location filter to the CSV export", at: minutesAgo(12) };
    const html = card(chain({}, { status: "working" }), {
      activity: [activity("f", { working_on: request })],
      glances: [glance("f", { step: "Running bundle exec rspec spec/exports" })],
    });
    expect(html).toContain("Working · 12 min");
    expect(html).toContain("Add a location filter to the CSV export");
    expect(html).toContain("Running bundle exec rspec spec/exports");
    expect(html).not.toContain("Mark done");
  });

  it("a limited card says when the limit resets, and offers Try now", () => {
    const resets_at = "2026-11-04T15:00:00.000Z";
    const html = card(chain({}, { status: "limited" }), {
      activity: [activity("f", { working_on: { text: "Add a filter", at: minutesAgo(5) } })],
      limits: [{ provider: "claude", resets_at }],
    });
    expect(html).toContain(`Waiting for the usage limit · resets ${formatReset(resets_at, NOW, "en")}`);
    expect(html).toContain("Add a filter");
    expect(html).toContain(">Try now</button>");
  });

  it("a limited card without a reset time, or in the grace without a hold, says no time", () => {
    const unknown = card(chain({}, { status: "limited" }), { limits: [{ provider: "claude", resets_at: null }] });
    expect(unknown).toContain("Waiting for the usage limit");
    expect(unknown).not.toContain("resets");
    const grace = card(chain({}, { status: "limited" }));
    expect(grace).toContain("Waiting for the usage limit");
    expect(grace).not.toContain("resets");
    expect(grace).toContain(">Try now</button>");
  });

  it("a running card without a screen line shows the latest message a member sent", () => {
    const html = card(chain({}, { status: "working" }), {
      activity: [
        activity("f", {
          working_on: { text: "", at: minutesAgo(2) },
          last_sent: { recipient: "b3", text: "Write request specs for the filter", at: minutesAgo(1) },
        }),
      ],
    });
    expect(html).toContain("@feature → @testing");
    expect(html).toContain("Write request specs for the filter");
    expect(html).not.toContain('class="task-request"');
  });

  it("a card waiting on background work says so in place of its request", () => {
    const html = card([s("f", "b2")], { glances: [glance("f", { background_work: true })] });
    expect(html).toContain("Waiting for its background work");
    expect(html).not.toContain('class="task-request"');
  });

  it("a finished card shows the reply, its files, the unread dot, Open and Mark done", () => {
    const html = card(chain(), {
      activity: [
        activity("m", {
          last_reply: {
            message_id: "r1",
            text: "Done. All green.",
            files: ["report.md"],
            at: minutesAgo(25),
            raw: false,
          },
          last_for_user_at: minutesAgo(25),
        }),
      ],
    });
    expect(html).toContain("Done. All green.");
    expect(html).toContain("report.md");
    expect(html).toContain('aria-label="Unread"');
    expect(html).toContain(">Open</button>");
    expect(html).toContain(">Mark done</button>");
    expect(html).toContain("Done · 30 min ago");
  });

  it("a finished card whose final answer is raw output says so in place of the terminal's text", () => {
    const banner = "▐▛███▛█   Claude Code v2.1.318";
    const html = card([s("m", "b1")], {
      activity: [activity("m", { last_reply: { message_id: "r1", text: banner, files: [], at: minutesAgo(25), raw: true } })],
    });
    expect(html).toContain("Raw output (the bot finished without sending a reply)");
    expect(html).not.toContain(banner);
  });

  it("a read finished card has no unread dot", () => {
    const reply = { message_id: "r1", text: "Done.", files: [], at: minutesAgo(25), raw: false };
    const html = card([s("m", "b1", { seen_at: minutesAgo(1) })], {
      activity: [activity("m", { last_reply: reply, last_for_user_at: minutesAgo(25) })],
    });
    expect(html).not.toContain('aria-label="Unread"');
  });

  it("a finished card says which bot still owes its result", () => {
    const html = card(chain({}, { open_request_id: "r1" }));
    expect(html).toContain("@feature stopped before sending its result");
  });

  it("a recurring run shows its chip", () => {
    const html = card([s("m", "b1")], { activity: [activity("m", { run_of: "Weekly review" })] });
    expect(html).toContain(">Recurring</span>");
    expect(html).toContain('title="Weekly review"');
    expect(card([s("m", "b1")])).not.toContain("Recurring");
  });

  it("a trail past four shows the holder in the last place and how many more", () => {
    const sessions = [
      s("a", "b1"),
      s("b", "b2", { opened_by: "a" }),
      s("c", "b3", { opened_by: "b" }),
      s("d", "b2", { opened_by: "c", number: 2 }),
      s("e", "b3", { opened_by: "d", number: 2, status: "working" }),
      s("f", "b3", { opened_by: "e", number: 3 }),
    ];
    const html = card(sessions);
    expect(html).toContain("@feature · 1");
    expect(html).not.toContain("@feature · 2");
    expect(html).toContain("@testing · 2");
    expect(html).toContain("+2");
  });
});

const run = (extra: Partial<RunGlance> = {}): RunGlance => ({
  run_id: "r1",
  name: "Ticket to merge request",
  status: "running",
  step: "Review",
  handle: "testing",
  visit: 2,
  pause: null,
  outcomes: [],
  holder: null,
  ...extra,
});

describe("a workflow run's card", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("a running run's card says its workflow, step and round", () => {
    const html = card(chain({}, {}, { status: "working" }), { activity: [activity("t", { workflow: run() })] });
    expect(html).toContain("Ticket to merge request · Review (@testing) · round 2");
    expect(html).not.toContain("Working ·");
    expect(html).not.toContain("Stop the run");
  });

  it("a running run's card keeps the usage-limit wait, a queue and paused bots in its state", () => {
    const resets_at = "2026-11-04T15:00:00.000Z";
    const line = "Ticket to merge request · Review (@testing) · round 2";
    const limited = card(chain({}, {}, { status: "limited" }), {
      activity: [activity("t", { workflow: run() })],
      limits: [{ provider: "claude", resets_at }],
    });
    expect(limited).toContain(`Waiting for the usage limit · resets ${formatReset(resets_at, NOW, "en")}`);
    expect(limited).toContain(">Try now</button>");
    expect(limited).not.toContain(line);
    const queued = [activity("t", { workflow: run(), queued: 1 })];
    expect(card(chain(), { activity: queued })).toContain(">Queued</span>");
    expect(card(chain(), { activity: queued, paused: true })).toContain(">Paused</span>");
    expect(card(chain({}, {}, { status: "starting" }), { activity: [activity("t", { workflow: run() })] })).toContain(
      line,
    );
    const background = card([s("t", "b3")], {
      activity: [activity("t", { workflow: run() })],
      glances: [glance("t", { background_work: true })],
    });
    expect(background).toContain(line);
  });

  it("a running run with no member at work stays in Running and waits for its step's bot", () => {
    const html = card(chain(), { activity: [activity("t", { workflow: run() })] });
    expect(html).toContain('class="task-card running"');
    expect(html).toContain("Waiting for @testing's result");
    expect(html).not.toContain("Mark done");
  });

  it("a paused run's card says why, in Needs you", () => {
    const paused = run({ status: "paused", pause: { kind: "no_result", step_id: "s2" }, holder: "t" });
    const html = card(chain(), { activity: [activity("t", { workflow: paused })] });
    expect(html).toContain('class="task-card needs_you"');
    expect(html).toContain("Review ended without choosing an outcome");
    expect(html).toContain("Ticket to merge request · Review (@testing) · round 2");
  });
});

describe("the pause buttons call their commands", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  const onError = vi.fn();
  let host: HTMLElement;
  let root: Root;
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    tauri.invoked = [];
    tauri.answer = null;
    tauri.asked = [];
    tauri.confirm = true;
    onError.mockReset();
  });

  async function mount(glance: RunGlance) {
    const sessions = chain();
    const { task, activity: acts, glances } = taskOf(sessions, { activity: [activity("t", { workflow: glance })] });
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    await act(async () =>
      root.render(
        <TaskCard
          task={task}
          bots={bots}
          sessions={sessions}
          activity={acts}
          glances={glances}
          now={NOW}
          onOpen={() => {}}
          onMarkDone={() => {}}
          onError={onError}
        />,
      ),
    );
  }
  const pauseButtons = () => Array.from(host.querySelectorAll<HTMLButtonElement>(".workflow-pause button"));
  const press = (label: string) =>
    act(async () => {
      const button = pauseButtons().find((b) => b.textContent === label);
      expect(button, label).toBeDefined();
      button!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

  it("visit cap: Another round and Stop the run", async () => {
    await mount(run({ status: "paused", pause: { kind: "visit_cap", step_id: "s2" }, visit: 3, holder: "t" }));
    expect(host.textContent).toContain("Review already ran 3 times");
    expect(pauseButtons().map((b) => b.textContent)).toEqual(["Another round", "Stop the run"]);
    await press("Another round");
    await press("Stop the run");
    expect(tauri.invoked).toEqual([
      ["another_round", { runId: "r1" }],
      ["stop_run", { runId: "r1" }],
    ]);
  });

  it("no result: one button per outcome, Remind and Stop the run", async () => {
    const outcomes = ["approved", "changes requested"];
    await mount(run({ status: "paused", pause: { kind: "no_result", step_id: "s2" }, outcomes, holder: "t" }));
    expect(host.textContent).toContain("Review ended without choosing an outcome");
    expect(pauseButtons().map((b) => b.textContent)).toEqual([...outcomes, "Remind @testing", "Stop the run"]);
    await press("changes requested");
    await press("Remind @testing");
    await press("Stop the run");
    expect(tauri.invoked).toEqual([
      ["pick_outcome", { runId: "r1", outcome: "changes requested" }],
      ["remind_step", { runId: "r1" }],
      ["stop_run", { runId: "r1" }],
    ]);
  });

  it("no result without outcomes: Go on", async () => {
    await mount(run({ status: "paused", pause: { kind: "no_result", step_id: "s2" }, holder: "t" }));
    expect(pauseButtons().map((b) => b.textContent)).toEqual(["Go on", "Remind @testing", "Stop the run"]);
    await press("Go on");
    expect(tauri.invoked).toEqual([["pick_outcome", { runId: "r1", outcome: null }]]);
  });

  it("bot missing: only Stop the run", async () => {
    await mount(run({ status: "paused", pause: { kind: "bot_missing", step_id: "s2" }, handle: null, holder: null }));
    expect(host.textContent).toContain("Step Review has no bot");
    expect(pauseButtons().map((b) => b.textContent)).toEqual(["Stop the run"]);
    await press("Stop the run");
    expect(tauri.invoked).toEqual([["stop_run", { runId: "r1" }]]);
    expect(onError).not.toHaveBeenCalled();
  });

  it("Stop the run asks first, and stops the run only on yes", async () => {
    await mount(run({ status: "paused", pause: { kind: "visit_cap", step_id: "s2" }, visit: 3, holder: "t" }));
    tauri.confirm = false;
    await press("Stop the run");
    expect(tauri.asked).toEqual(["Stop Ticket to merge request? A stopped run cannot be resumed."]);
    expect(tauri.invoked).toEqual([]);
    tauri.confirm = true;
    await press("Stop the run");
    expect(tauri.asked).toHaveLength(2);
    expect(tauri.invoked).toEqual([["stop_run", { runId: "r1" }]]);
  });

  it("a call in flight disables the buttons, and a refusal goes to onError", async () => {
    let refuse: (e: unknown) => void = () => {};
    tauri.answer = () => new Promise((_, reject) => (refuse = reject));
    await mount(run({ status: "paused", pause: { kind: "visit_cap", step_id: "s2" }, holder: "t" }));
    await press("Another round");
    expect(pauseButtons().map((b) => b.disabled)).toEqual([true, true]);
    await act(async () => refuse({ code: "workflowRunNotPaused" }));
    expect(onError).toHaveBeenCalledWith({ code: "workflowRunNotPaused" });
    expect(pauseButtons().map((b) => b.disabled)).toEqual([false, false]);
  });
});
