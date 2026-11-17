import dagre from "@dagrejs/dagre";
import type { Edge, Node } from "@xyflow/react";
import type {
  BotView,
  NewWorkflow,
  RunGlance,
  Visit,
  WorkflowArrow,
  WorkflowDef,
  WorkflowPos,
  WorkflowStep,
} from "./api";
import type { Translate } from "./state";

export const LIMITS = {
  name: 40,
  when: 300,
  steps: 30,
  stepName: 40,
  arrows: 6,
  outcome: 60,
  visitCap: 3,
};

const FORMAT = 1;
const START = "start";
const DONE = "done";

const STEP_W = 200;
const STEP_H = 64;
const END_W = 120;
const END_H = 48;
const RANKSEP = 80;
const NODESEP = 60;
export const BACK_SIDE = 140;
const MAP_PAD = 24;
const LABEL_HALF = 64;

export interface WorkflowProblem {
  code: string;
  params: Record<string, string>;
  stepId?: string;
  arrowIndex?: number;
}

export function workflowProblems(
  input: NewWorkflow,
  botIds: Set<string>,
  otherNames: string[],
): WorkflowProblem[] {
  const out: WorkflowProblem[] = [];
  const add = (
    code: string,
    params: Record<string, string> = {},
    stepId?: string,
    arrowIndex?: number,
  ) => {
    const p: WorkflowProblem = { code, params };
    if (stepId !== undefined) p.stepId = stepId;
    if (arrowIndex !== undefined) p.arrowIndex = arrowIndex;
    out.push(p);
  };
  const def = input.def;
  const ids = new Set<string>();
  let badDefinition = def.version !== FORMAT;
  for (const s of def.steps) {
    if (s.id === "" || s.id === START || s.id === DONE || ids.has(s.id))
      badDefinition = true;
    ids.add(s.id);
  }
  if (badDefinition) add("workflowBadDefinition");

  const name = input.name.trim();
  if (name === "") add("workflowNameRequired");
  if ([...input.name].length > LIMITS.name)
    add("workflowNameTooLong", { max: String(LIMITS.name) });
  if (
    name !== "" &&
    otherNames.some((n) => n.trim().toLowerCase() === name.toLowerCase())
  ) {
    add("workflowNameTaken", { name });
  }
  if ([...input.when_to_use].length > LIMITS.when)
    add("workflowWhenTooLong", { max: String(LIMITS.when) });
  if (def.steps.length === 0) add("workflowNoSteps");
  if (def.steps.length > LIMITS.steps)
    add("workflowTooManySteps", { max: String(LIMITS.steps) });

  const names = new Set<string>();
  for (const s of def.steps) {
    const n = s.name.trim();
    if (n === "") add("workflowStepNameRequired", {}, s.id);
    if ([...n].length > LIMITS.stepName)
      add(
        "workflowStepNameTooLong",
        { step: n, max: String(LIMITS.stepName) },
        s.id,
      );
    if (n.toLowerCase() === DONE) add("workflowStepNameReserved", {}, s.id);
    if (n !== "") {
      if (names.has(n.toLowerCase()))
        add("workflowStepNameTaken", { name: n }, s.id);
      names.add(n.toLowerCase());
    }
    if (!botIds.has(s.bot_id)) add("workflowStepNoBot", { step: n }, s.id);
    if (s.instructions.trim() === "")
      add("workflowStepInstructionsRequired", { step: n }, s.id);
    s.arrows.forEach((a, i) => {
      if (a.to !== DONE && !ids.has(a.to))
        add("workflowArrowTarget", { step: n }, s.id, i);
    });
    if (s.arrows.length > 1) {
      const seen = new Set<string>();
      s.arrows.forEach((a, i) => {
        const o = (a.outcome ?? "").trim();
        if (o === "")
          return add("workflowOutcomeRequired", { step: n }, s.id, i);
        if ([...o].length > LIMITS.outcome)
          add(
            "workflowOutcomeTooLong",
            { step: n, max: String(LIMITS.outcome) },
            s.id,
            i,
          );
        if (/[\n\r]/.test(o)) add("workflowOutcomeLine", { step: n }, s.id, i);
        if (seen.has(o.toLowerCase()))
          add("workflowOutcomeTaken", { step: n, outcome: o }, s.id, i);
        seen.add(o.toLowerCase());
      });
    }
    if (s.arrows.length > LIMITS.arrows)
      add(
        "workflowTooManyArrows",
        { step: n, max: String(LIMITS.arrows) },
        s.id,
      );
  }

  const first = def.steps.find((s) => s.id === def.start);
  if (!first) {
    add("workflowNoStart");
    return out;
  }
  const seen = new Set<string>([first.id]);
  const queue = [first];
  for (let s = queue.shift(); s; s = queue.shift()) {
    for (const a of s.arrows) {
      const next = def.steps.find((x) => x.id === a.to);
      if (next && !seen.has(next.id)) {
        seen.add(next.id);
        queue.push(next);
      }
    }
  }
  for (const s of def.steps) {
    if (!seen.has(s.id))
      add("workflowUnreachable", { step: s.name.trim() }, s.id);
  }
  return out;
}

export function newStepId(def: WorkflowDef): string {
  const used = new Set(def.steps.map((s) => s.id));
  let k = 1;
  while (used.has(`s${k}`)) k++;
  return `s${k}`;
}

export function addStep(
  def: WorkflowDef,
  botId: string,
  pos: WorkflowPos,
): WorkflowDef {
  const id = newStepId(def);
  const step: WorkflowStep = {
    id,
    name: `Step ${id.slice(1)}`,
    bot_id: botId,
    instructions: "",
    arrows: [],
    pos,
  };
  return { ...def, steps: [...def.steps, step] };
}

export function connect(
  def: WorkflowDef,
  from: string,
  to: string,
): WorkflowDef {
  if (from === START) return to === DONE ? def : { ...def, start: to };
  return {
    ...def,
    steps: def.steps.map((s) =>
      s.id !== from || s.arrows.some((a) => a.to === to)
        ? s
        : { ...s, arrows: [...s.arrows, { to, outcome: null }] },
    ),
  };
}

export function removeStep(def: WorkflowDef, id: string): WorkflowDef {
  return {
    ...def,
    start: def.start === id ? null : def.start,
    steps: def.steps
      .filter((s) => s.id !== id)
      .map((s) => ({ ...s, arrows: s.arrows.filter((a) => a.to !== id) })),
  };
}

function withArrows(
  def: WorkflowDef,
  from: string,
  change: (arrows: WorkflowArrow[]) => WorkflowArrow[],
): WorkflowDef {
  return {
    ...def,
    steps: def.steps.map((s) =>
      s.id === from ? { ...s, arrows: change(s.arrows) } : s,
    ),
  };
}

export function removeArrow(
  def: WorkflowDef,
  from: string,
  index: number,
): WorkflowDef {
  return withArrows(def, from, (arrows) =>
    arrows.filter((_, i) => i !== index),
  );
}

export function setOutcome(
  def: WorkflowDef,
  from: string,
  index: number,
  text: string,
): WorkflowDef {
  return withArrows(def, from, (arrows) =>
    arrows.map((a, i) => (i === index ? { ...a, outcome: text } : a)),
  );
}

export function movedTo(
  def: WorkflowDef,
  nodeId: string,
  pos: WorkflowPos,
): WorkflowDef {
  if (nodeId === START) return { ...def, start_pos: pos };
  if (nodeId === DONE) return { ...def, done_pos: pos };
  return {
    ...def,
    steps: def.steps.map((s) => (s.id === nodeId ? { ...s, pos } : s)),
  };
}

export function tidy(def: WorkflowDef): WorkflowDef {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "TB", ranksep: RANKSEP, nodesep: NODESEP });
  g.setDefaultEdgeLabel(() => ({}));
  g.setNode(START, { width: END_W, height: END_H });
  g.setNode(DONE, { width: END_W, height: END_H });
  for (const s of def.steps) g.setNode(s.id, { width: STEP_W, height: STEP_H });
  const known = new Set(def.steps.map((s) => s.id));
  if (def.start && known.has(def.start)) g.setEdge(START, def.start);
  for (const s of def.steps) {
    for (const a of s.arrows)
      if (a.to === DONE || known.has(a.to)) g.setEdge(s.id, a.to);
  }
  dagre.layout(g);
  const at = (id: string, w: number, h: number): WorkflowPos => {
    const n = g.node(id);
    return { x: Math.round(n.x - w / 2), y: Math.round(n.y - h / 2) };
  };
  return {
    ...def,
    start_pos: at(START, END_W, END_H),
    done_pos: at(DONE, END_W, END_H),
    steps: def.steps.map((s) => ({ ...s, pos: at(s.id, STEP_W, STEP_H) })),
  };
}

export function needsLayout(def: WorkflowDef): boolean {
  return (
    def.start_pos === null ||
    def.done_pos === null ||
    def.steps.some((s) => s.pos === null)
  );
}

export function placeMissing(def: WorkflowDef): WorkflowDef {
  const anyPlaced =
    def.start_pos !== null ||
    def.done_pos !== null ||
    def.steps.some((s) => s.pos !== null);
  if (!anyPlaced) return tidy(def);

  const placed = new Map<string, { pos: WorkflowPos; w: number; h: number }>();
  const note = (id: string, pos: WorkflowPos | null, w: number, h: number) => {
    if (pos) placed.set(id, { pos, w, h });
  };
  note(START, def.start_pos, END_W, END_H);
  note(DONE, def.done_pos, END_W, END_H);
  for (const s of def.steps) note(s.id, s.pos, STEP_W, STEP_H);

  const free = (p: WorkflowPos, w: number, h: number): WorkflowPos => {
    const q = { ...p };
    const overlaps = () =>
      [...placed.values()].some(
        (n) =>
          q.x < n.pos.x + n.w &&
          n.pos.x < q.x + w &&
          q.y < n.pos.y + n.h &&
          n.pos.y < q.y + h,
      );
    while (overlaps()) q.x += w + NODESEP;
    return q;
  };
  const put = (
    id: string,
    p: WorkflowPos,
    w: number,
    h: number,
  ): WorkflowPos => {
    const pos = free(p, w, h);
    placed.set(id, { pos, w, h });
    return pos;
  };

  const steps = def.steps.map((s) => {
    if (s.pos) return s;
    const from = def.steps.find(
      (p) => placed.has(p.id) && p.arrows.some((a) => a.to === s.id),
    );
    const pred = from
      ? placed.get(from.id)
      : def.start === s.id
        ? placed.get(START)
        : undefined;
    if (pred)
      return {
        ...s,
        pos: put(
          s.id,
          {
            x: pred.pos.x + (pred.w - STEP_W) / 2,
            y: pred.pos.y + pred.h + RANKSEP,
          },
          STEP_W,
          STEP_H,
        ),
      };
    const nodes = [...placed.values()];
    const x = Math.max(...nodes.map((n) => n.pos.x + n.w)) + NODESEP;
    const y = Math.min(...nodes.map((n) => n.pos.y));
    return { ...s, pos: put(s.id, { x, y }, STEP_W, STEP_H) };
  });

  let start_pos = def.start_pos;
  if (!start_pos) {
    const first = steps.find((s) => s.id === def.start)?.pos ?? steps[0]?.pos;
    start_pos = put(
      START,
      {
        x: (first?.x ?? 0) + (STEP_W - END_W) / 2,
        y: (first?.y ?? 0) - END_H - RANKSEP,
      },
      END_W,
      END_H,
    );
  }
  let done_pos = def.done_pos;
  if (!done_pos) {
    const nodes = [...placed.values()].filter((n) => n.pos !== def.done_pos);
    const bottom = Math.max(...nodes.map((n) => n.pos.y + n.h));
    const last = nodes.find((n) => n.pos.y + n.h === bottom)!;
    done_pos = put(
      DONE,
      { x: last.pos.x + (last.w - END_W) / 2, y: bottom + RANKSEP },
      END_W,
      END_H,
    );
  }
  return { ...def, steps, start_pos, done_pos };
}

export interface MapBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function mapBounds(def: WorkflowDef): MapBox {
  const boxes = new Map<string, MapBox>();
  const note = (id: string, pos: WorkflowPos | null, w: number, h: number) => {
    if (pos) boxes.set(id, { x: pos.x, y: pos.y, width: w, height: h });
  };
  note(START, def.start_pos, END_W, END_H);
  note(DONE, def.done_pos, END_W, END_H);
  for (const s of def.steps) note(s.id, s.pos, STEP_W, STEP_H);
  const all = [...boxes.values()];
  if (all.length === 0)
    return {
      x: -MAP_PAD,
      y: -MAP_PAD,
      width: 2 * MAP_PAD,
      height: 2 * MAP_PAD,
    };
  const left = Math.min(...all.map((b) => b.x));
  const top = Math.min(...all.map((b) => b.y));
  let right = Math.max(...all.map((b) => b.x + b.width));
  const bottom = Math.max(...all.map((b) => b.y + b.height));
  for (const s of def.steps) {
    const from = boxes.get(s.id);
    for (const a of s.arrows) {
      const to = boxes.get(a.to);
      if (!from || !to || to.y >= from.y + from.height) continue;
      const side =
        Math.max(from.x + from.width / 2, to.x + to.width / 2) + BACK_SIDE;
      right = Math.max(right, side + (a.outcome ? LABEL_HALF : 0));
    }
  }
  return {
    x: left - MAP_PAD,
    y: top - MAP_PAD,
    width: right - left + 2 * MAP_PAD,
    height: bottom - top + 2 * MAP_PAD,
  };
}

export function stepCentre(def: WorkflowDef, id: string): WorkflowPos | null {
  const pos = def.steps.find((s) => s.id === id)?.pos;
  return pos ? { x: pos.x + STEP_W / 2, y: pos.y + STEP_H / 2 } : null;
}

export function mapViewport(
  bounds: MapBox,
  focus: WorkflowPos | null,
  view: { width: number; height: number },
): { x: number; y: number; zoom: number } {
  const zoom = Math.min(1, view.width / bounds.width);
  const x = (view.width - bounds.width * zoom) / 2 - bounds.x * zoom;
  const tall = bounds.height * zoom;
  if (tall <= view.height)
    return { x, y: (view.height - tall) / 2 - bounds.y * zoom, zoom };
  const top = -bounds.y * zoom;
  const end = view.height - (bounds.y + bounds.height) * zoom;
  const want = focus ? view.height / 2 - focus.y * zoom : top;
  return { x, y: Math.min(top, Math.max(end, want)), zoom };
}

export interface FlowNodeData extends Record<string, unknown> {
  name: string;
  handle: string;
  color: string;
  problem: boolean;
}

export function toFlow(
  def: WorkflowDef,
  bots: BotView[],
  problems: WorkflowProblem[],
): { nodes: Node<FlowNodeData>[]; edges: Edge[] } {
  const bad = new Set(
    problems
      .map((p) => p.stepId)
      .filter((id): id is string => id !== undefined),
  );
  const zero = { x: 0, y: 0 };
  const end = (id: string, pos: WorkflowPos | null): Node<FlowNodeData> => ({
    id,
    type: id,
    position: pos ?? zero,
    data: { name: "", handle: "", color: "", problem: false },
  });
  const nodes: Node<FlowNodeData>[] = [
    end(START, def.start_pos),
    end(DONE, def.done_pos),
  ];
  for (const s of def.steps) {
    const bot = bots.find((b) => b.id === s.bot_id);
    nodes.push({
      id: s.id,
      type: "step",
      position: s.pos ?? zero,
      data: {
        name: s.name,
        handle: bot?.handle ?? "",
        color: bot?.color ?? "",
        problem: bad.has(s.id),
      },
    });
  }
  const edges: Edge[] = [];
  if (def.start)
    edges.push({ id: `${START}:0`, source: START, target: def.start });
  for (const s of def.steps) {
    s.arrows.forEach((a, i) => {
      const edge: Edge = { id: `${s.id}:${i}`, source: s.id, target: a.to };
      if (a.outcome) edge.label = a.outcome;
      edges.push(edge);
    });
  }
  return { nodes, edges };
}

export function runMapState(
  def: WorkflowDef,
  visits: Visit[],
): {
  steps: Record<string, { visits: number; current: boolean }>;
  taken: Set<string>;
} {
  const ordered = [...visits].sort((a, b) => a.ord - b.ord);
  const steps: Record<string, { visits: number; current: boolean }> = {};
  const taken = new Set<string>();
  const arrowOf = (v: Visit, to: string | null): string | null => {
    const s = def.steps.find((x) => x.id === v.step_id);
    if (!s) return null;
    let i = -1;
    if (s.arrows.length === 1) i = 0;
    else if (v.outcome !== null) {
      const o = v.outcome.trim().toLowerCase();
      i = s.arrows.findIndex(
        (a) => (a.outcome ?? "").trim().toLowerCase() === o,
      );
    }
    if (i < 0 && to !== null) i = s.arrows.findIndex((a) => a.to === to);
    return i < 0 ? null : `${s.id}:${i}`;
  };
  ordered.forEach((v, k) => {
    const entry = (steps[v.step_id] ??= { visits: 0, current: false });
    entry.visits += 1;
    if (v.ended_at === null) entry.current = true;
    if (k === 0 && def.start === v.step_id) taken.add(`${START}:0`);
    const next = ordered[k + 1];
    if (next) {
      const id = arrowOf(v, next.step_id);
      if (id) taken.add(id);
    } else if (v.ended_at !== null) {
      const id = arrowOf(v, DONE);
      const s = def.steps.find((x) => x.id === v.step_id);
      if (id && s && s.arrows[Number(id.split(":")[1])]?.to === DONE)
        taken.add(id);
    }
  });
  return { steps, taken };
}

export function glanceLine(g: RunGlance, t: Translate): string {
  if (!g.step) return g.name;
  const key = g.visit > 1 ? "workflow.cardLine" : "workflow.cardLineFirst";
  const who = g.handle ? `${g.step} (@${g.handle})` : g.step;
  return t(key, { name: g.name, step: who, visit: g.visit });
}
