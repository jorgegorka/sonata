import i18next, { type TFunction } from "i18next";
import { beforeAll, describe, expect, it } from "vitest";
import de from "../locales/de.json";
import en from "../locales/en.json";
import es from "../locales/es.json";
import fr from "../locales/fr.json";
import type {
  BotView,
  NewWorkflow,
  RunGlance,
  Visit,
  WorkflowDef,
  WorkflowStep,
} from "./api";
import {
  addStep,
  connect,
  glanceLine,
  LIMITS,
  mapBounds,
  mapViewport,
  movedTo,
  needsLayout,
  newStepId,
  placeMissing,
  removeArrow,
  removeStep,
  runMapState,
  setOutcome,
  stepCentre,
  tidy,
  toFlow,
  workflowProblems,
} from "./workflows";

let t: TFunction;
let tEs: TFunction;
beforeAll(async () => {
  const make = (lng: string) =>
    i18next.createInstance().init({
      lng,
      fallbackLng: "en",
      resources: {
        en: { translation: en },
        es: { translation: es },
        fr: { translation: fr },
        de: { translation: de },
      },
      interpolation: { escapeValue: false },
    });
  t = await make("en");
  tEs = await make("es");
});

const step = (
  id: string,
  name: string,
  bot: string,
  arrows: WorkflowStep["arrows"] = [],
): WorkflowStep => ({
  id,
  name,
  bot_id: bot,
  instructions: "Do it.",
  arrows,
  pos: null,
});
const arrow = (to: string, outcome: string | null = null) => ({ to, outcome });

const ticket = (): NewWorkflow => ({
  name: "Ticket to merge request",
  when_to_use: "A ticket to build.",
  def: {
    version: 1,
    start: "plan",
    steps: [
      step("plan", "Plan", "b-plan", [arrow("implement")]),
      step("implement", "Implement", "b-feature", [arrow("review")]),
      step("review", "Review", "b-review", [
        arrow("test", "approved"),
        arrow("implement", "changes requested"),
      ]),
      step("test", "Test", "b-testing", [
        arrow("done", "passed"),
        arrow("implement", "failed"),
      ]),
    ],
    start_pos: null,
    done_pos: null,
  },
});
const BOTS = new Set(["b-plan", "b-feature", "b-review", "b-testing"]);

describe("workflowProblems", () => {
  it("passes the ticket workflow", () => {
    expect(workflowProblems(ticket(), BOTS, [])).toEqual([]);
  });

  it("problems mirror the Rust rules", () => {
    type Change = (w: NewWorkflow) => void;
    const cases: [Change, string, string | undefined][] = [
      [(w) => (w.name = ""), "workflowNameRequired", undefined],
      [(w) => (w.name = "x".repeat(41)), "workflowNameTooLong", undefined],
      [
        (w) => (w.when_to_use = "x".repeat(301)),
        "workflowWhenTooLong",
        undefined,
      ],
      [(w) => (w.def.steps = []), "workflowNoSteps", undefined],
      [
        (w) => {
          for (let i = 0; i < 27; i++)
            w.def.steps.push(step(`x${i}`, `X${i}`, "b-plan"));
        },
        "workflowTooManySteps",
        undefined,
      ],
      [(w) => (w.def.steps[0].name = ""), "workflowStepNameRequired", "plan"],
      [
        (w) => (w.def.steps[0].name = "x".repeat(41)),
        "workflowStepNameTooLong",
        "plan",
      ],
      [
        (w) => (w.def.steps[0].name = "DONE"),
        "workflowStepNameReserved",
        "plan",
      ],
      [
        (w) => (w.def.steps[1].name = "PLAN"),
        "workflowStepNameTaken",
        "implement",
      ],
      [
        (w) => (w.def.steps[2].bot_id = "missing"),
        "workflowStepNoBot",
        "review",
      ],
      [
        (w) => (w.def.steps[2].instructions = "  "),
        "workflowStepInstructionsRequired",
        "review",
      ],
      [(w) => (w.def.start = null), "workflowNoStart", undefined],
      [(w) => (w.def.start = "nope"), "workflowNoStart", undefined],
      [
        (w) => (w.def.steps[2].arrows[0].to = "nope"),
        "workflowArrowTarget",
        "review",
      ],
      [
        (w) => (w.def.steps[2].arrows[1].outcome = null),
        "workflowOutcomeRequired",
        "review",
      ],
      [
        (w) => (w.def.steps[2].arrows[0].outcome = "x".repeat(61)),
        "workflowOutcomeTooLong",
        "review",
      ],
      [
        (w) => (w.def.steps[2].arrows[0].outcome = "a\nb"),
        "workflowOutcomeLine",
        "review",
      ],
      [
        (w) => {
          w.def.steps[2].arrows[0].outcome = "Approved";
          w.def.steps[2].arrows[1].outcome = "approved";
        },
        "workflowOutcomeTaken",
        "review",
      ],
      [
        (w) =>
          (w.def.steps[2].arrows = Array.from({ length: 7 }, (_, i) =>
            arrow("implement", `o${i}`),
          )),
        "workflowTooManyArrows",
        "review",
      ],
      [
        (w) => w.def.steps.push(step("lonely", "Lonely", "b-plan")),
        "workflowUnreachable",
        "lonely",
      ],
      [(w) => (w.def.version = 2), "workflowBadDefinition", undefined],
      [(w) => (w.def.steps[1].id = "done"), "workflowBadDefinition", undefined],
      [(w) => (w.def.steps[1].id = ""), "workflowBadDefinition", undefined],
    ];
    cases.forEach(([change, code, stepId], i) => {
      const w = ticket();
      change(w);
      const found = workflowProblems(w, BOTS, []).filter(
        (p) => p.code === code,
      );
      expect(found.length, `case ${i} ${code}`).toBeGreaterThan(0);
      if (stepId) expect(found[0].stepId, `case ${i}`).toBe(stepId);
    });
  });

  it("returns every problem, with the params the texts need", () => {
    const w = ticket();
    w.name = "";
    w.def.steps[0].instructions = "";
    w.def.steps[2].arrows[1].outcome = null;
    const problems = workflowProblems(w, BOTS, []);
    expect(problems.map((p) => p.code)).toEqual([
      "workflowNameRequired",
      "workflowStepInstructionsRequired",
      "workflowOutcomeRequired",
    ]);
    expect(problems[1].params).toEqual({ step: "Plan" });
    expect(problems[2]).toMatchObject({ stepId: "review", arrowIndex: 1 });
    const long = ticket();
    long.name = "x".repeat(41);
    expect(workflowProblems(long, BOTS, [])[0].params).toEqual({ max: "40" });
  });

  it("refuses a name another workflow has, ignoring case", () => {
    const problems = workflowProblems(ticket(), BOTS, [
      "ticket TO merge request",
    ]);
    expect(problems).toEqual([
      {
        code: "workflowNameTaken",
        params: { name: "Ticket to merge request" },
      },
    ]);
  });

  it("exposes the limits", () => {
    expect(LIMITS).toEqual({
      name: 40,
      when: 300,
      steps: 30,
      stepName: 40,
      arrows: 6,
      outcome: 60,
      visitCap: 3,
    });
  });
});

describe("editing", () => {
  it("newStepId takes the first free number and addStep names the step", () => {
    const def = ticket().def;
    expect(newStepId(def)).toBe("s1");
    const next = addStep(def, "b-plan", { x: 5, y: 6 });
    expect(next.steps[next.steps.length - 1]).toMatchObject({
      id: "s1",
      name: "Step 1",
      bot_id: "b-plan",
      pos: { x: 5, y: 6 },
    });
    expect(newStepId(next)).toBe("s2");
    expect(def.steps).toHaveLength(4);
  });

  it("connect from Start replaces the start, and a duplicate is a no-op", () => {
    const def = ticket().def;
    expect(connect(def, "start", "test").start).toBe("test");
    const once = connect(def, "plan", "test");
    expect(once.steps[0].arrows).toEqual([arrow("implement"), arrow("test")]);
    expect(connect(once, "plan", "test")).toEqual(once);
  });

  it("a second arrow makes both need an outcome", () => {
    const w = ticket();
    w.def = connect(w.def, "plan", "test");
    const problems = workflowProblems(w, BOTS, []);
    expect(
      problems
        .filter((p) => p.code === "workflowOutcomeRequired")
        .map((p) => p.arrowIndex),
    ).toEqual([0, 1]);
    w.def = setOutcome(setOutcome(w.def, "plan", 0, "small"), "plan", 1, "big");
    expect(workflowProblems(w, BOTS, [])).toEqual([]);
  });

  it("removeStep drops the arrows into it and clears the start", () => {
    const def = ticket().def;
    const next = removeStep(def, "implement");
    expect(next.steps.map((s) => s.id)).toEqual(["plan", "review", "test"]);
    expect(next.steps[0].arrows).toEqual([]);
    expect(next.steps[1].arrows).toEqual([arrow("test", "approved")]);
    expect(removeStep(def, "plan").start).toBeNull();
  });

  it("removeArrow and movedTo", () => {
    const def = ticket().def;
    expect(removeArrow(def, "review", 0).steps[2].arrows).toEqual([
      arrow("implement", "changes requested"),
    ]);
    expect(movedTo(def, "plan", { x: 1, y: 2 }).steps[0].pos).toEqual({
      x: 1,
      y: 2,
    });
    expect(movedTo(def, "start", { x: 1, y: 2 }).start_pos).toEqual({
      x: 1,
      y: 2,
    });
    expect(movedTo(def, "done", { x: 3, y: 4 }).done_pos).toEqual({
      x: 3,
      y: 4,
    });
  });
});

describe("layout", () => {
  it("tidy places Start above the first step and Done below the last", () => {
    const def = tidy(ticket().def);
    expect(needsLayout(ticket().def)).toBe(true);
    expect(needsLayout(def)).toBe(false);
    const plan = def.steps[0].pos!;
    const test = def.steps[3].pos!;
    expect(def.start_pos!.y).toBeLessThan(plan.y);
    expect(def.done_pos!.y).toBeGreaterThan(test.y);
    expect(def.steps[1].pos!.y).toBeGreaterThan(plan.y);
  });

  it("placeMissing keeps existing positions and places the new step after its predecessor", () => {
    const tidied = tidy(ticket().def);
    const added: WorkflowDef = {
      ...tidied,
      steps: [...tidied.steps, step("s1", "Step 1", "b-plan")],
    };
    added.steps[3] = {
      ...added.steps[3],
      arrows: [...added.steps[3].arrows, arrow("s1", "other")],
    };
    const placed = placeMissing(added);
    expect(placed.steps.slice(0, 4).map((s) => s.pos)).toEqual(
      tidied.steps.map((s) => s.pos),
    );
    expect(placed.start_pos).toEqual(tidied.start_pos);
    const test = tidied.steps[3].pos!;
    expect(placed.steps[4].pos!.y).toBe(test.y + 64 + 80);
    expect(placed.steps[4].pos!.x).toBe(test.x + 200 + 60);
    expect(placed.steps[4].pos!.x).toBeGreaterThan(tidied.done_pos!.x + 120);
    expect(placed.done_pos).toEqual(tidied.done_pos);
    expect(needsLayout(placed)).toBe(false);
  });

  it("placeMissing places a step with no positioned predecessor right of the others", () => {
    const tidied = tidy(ticket().def);
    const added: WorkflowDef = {
      ...tidied,
      steps: [...tidied.steps, step("s1", "Step 1", "b-plan")],
    };
    const placed = placeMissing(added);
    const rightmost = Math.max(
      tidied.start_pos!.x + 120,
      tidied.done_pos!.x + 120,
      ...tidied.steps.map((s) => s.pos!.x + 200),
    );
    expect(placed.steps[4].pos!.x).toBeGreaterThan(rightmost);
    expect(placed.steps[4].pos!.y).toBe(tidied.start_pos!.y);
    expect(placed.steps.slice(0, 4).map((s) => s.pos)).toEqual(
      tidied.steps.map((s) => s.pos),
    );
  });

  it("placeMissing places a missing Done below the lowest node and Start above the start step", () => {
    const tidied = tidy(ticket().def);
    const placed = placeMissing({ ...tidied, start_pos: null, done_pos: null });
    const lowest = Math.max(...tidied.steps.map((s) => s.pos!.y));
    const plan = tidied.steps[0].pos!;
    expect(placed.done_pos!.y).toBeGreaterThan(lowest + 64);
    expect(placed.start_pos!.y + 48).toBeLessThan(plan.y);
    expect(placed.start_pos!.x + 60).toBe(plan.x + 100);
  });
});

describe("the map's view", () => {
  const column = (back: WorkflowStep["arrows"] = []): WorkflowDef => ({
    version: 1,
    start: "a",
    steps: [
      { ...step("a", "A", "b-plan", [arrow("b")]), pos: { x: 0, y: 128 } },
      {
        ...step("b", "B", "b-plan", [arrow("done"), ...back]),
        pos: { x: 0, y: 272 },
      },
    ],
    start_pos: { x: 40, y: 0 },
    done_pos: { x: 40, y: 416 },
  });

  it("mapBounds covers every box with room around", () => {
    expect(mapBounds(column())).toEqual({
      x: -24,
      y: -24,
      width: 200 + 48,
      height: 464 + 48,
    });
  });

  it("mapBounds widens for an arrow back, and its outcome", () => {
    expect(mapBounds(column([arrow("a")])).width).toBe(240 + 48);
    expect(mapBounds(column([arrow("a", "again")])).width).toBe(240 + 64 + 48);
  });

  it("stepCentre is the middle of a step's box", () => {
    expect(stepCentre(column(), "a")).toEqual({ x: 100, y: 160 });
    expect(stepCentre(column(), "nope")).toBeNull();
  });

  const tall = { x: -24, y: -24, width: 248, height: 512 };
  const drawer = { width: 520, height: 240 };

  it("mapViewport shows a narrow diagram at full size, centred across", () => {
    expect(mapViewport(tall, null, drawer)).toEqual({ x: 160, y: 24, zoom: 1 });
  });

  it("mapViewport shrinks a wide diagram to the box's width", () => {
    expect(mapViewport(tall, null, { width: 124, height: 240 })).toEqual({
      x: 12,
      y: 12,
      zoom: 0.5,
    });
  });

  it("mapViewport centres a diagram shorter than the box", () => {
    const short = { x: -24, y: -24, width: 248, height: 100 };
    expect(mapViewport(short, null, drawer).y).toBe(70 + 24);
  });

  it("mapViewport puts the focus in the middle, held within the diagram", () => {
    expect(mapViewport(tall, { x: 100, y: 160 }, drawer).y).toBe(120 - 160);
    expect(mapViewport(tall, { x: 100, y: 0 }, drawer).y).toBe(24);
    expect(mapViewport(tall, { x: 100, y: 440 }, drawer).y).toBe(240 - 488);
  });
});

describe("toFlow", () => {
  const bots = [
    { id: "b-plan", handle: "plan", color: "#123456" },
  ] as BotView[];

  it("maps steps, Start, Done and arrows to nodes and edges", () => {
    const w = ticket();
    const problems = workflowProblems(
      {
        ...w,
        def: {
          ...w.def,
          steps: [
            { ...w.def.steps[0], instructions: "" },
            ...w.def.steps.slice(1),
          ],
        },
      },
      BOTS,
      [],
    );
    const { nodes, edges } = toFlow(tidy(w.def), bots, problems);
    expect(nodes.map((n) => n.id)).toEqual([
      "start",
      "done",
      "plan",
      "implement",
      "review",
      "test",
    ]);
    expect(nodes.map((n) => n.type)).toEqual([
      "start",
      "done",
      "step",
      "step",
      "step",
      "step",
    ]);
    const plan = nodes.find((n) => n.id === "plan")!;
    expect(plan.data).toMatchObject({
      name: "Plan",
      handle: "plan",
      color: "#123456",
      problem: true,
    });
    expect(nodes.find((n) => n.id === "implement")!.data.problem).toBe(false);
    expect(edges.map((e) => e.id)).toContain("start:0");
    const e = edges.find((x) => x.id === "review:0")!;
    expect(e).toMatchObject({
      source: "review",
      target: "test",
      label: "approved",
    });
    expect(edges.find((x) => x.id === "test:0")!.target).toBe("done");
  });
});

describe("runMapState", () => {
  const visit = (
    ord: number,
    step_id: string,
    n: number,
    outcome: string | null,
    ended: boolean,
  ): Visit =>
    ({
      id: `v${ord}`,
      ord,
      step_id,
      n,
      outcome,
      ended_at: ended ? "2027-01-01T00:00:00Z" : null,
    }) as Visit;

  it("runMapState counts visits and marks the open one", () => {
    const def = ticket().def;
    const state = runMapState(def, [
      visit(1, "plan", 1, null, true),
      visit(2, "implement", 1, null, true),
      visit(3, "review", 1, "changes requested", true),
      visit(4, "implement", 2, null, true),
      visit(5, "review", 2, null, false),
    ]);
    expect(state.steps).toEqual({
      plan: { visits: 1, current: false },
      implement: { visits: 2, current: false },
      review: { visits: 2, current: true },
    });
    expect([...state.taken].sort()).toEqual([
      "implement:0",
      "plan:0",
      "review:1",
      "start:0",
    ]);
  });

  it("marks the arrow into Done once the last visit ends with it", () => {
    const state = runMapState(ticket().def, [
      visit(1, "test", 1, "passed", true),
    ]);
    expect(state.taken.has("test:0")).toBe(true);
  });
});

describe("glanceLine", () => {
  const glance = (over: Partial<RunGlance> = {}): RunGlance => ({
    run_id: "r1",
    name: "Flow",
    status: "running",
    step: "Review",
    handle: "review",
    visit: 2,
    pause: null,
    outcomes: [],
    holder: null,
    ...over,
  });

  it("names the step, its bot and the round", () => {
    expect(glanceLine(glance(), t)).toBe("Flow · Review (@review) · round 2");
    expect(glanceLine(glance({ visit: 1 }), t)).toBe("Flow · Review (@review)");
    expect(glanceLine(glance({ step: null, handle: null, visit: 0 }), t)).toBe(
      "Flow",
    );
    expect(glanceLine(glance(), tEs)).toBe("Flow · Review (@review) · ronda 2");
  });
});
