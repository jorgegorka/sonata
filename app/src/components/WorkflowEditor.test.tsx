// @vitest-environment happy-dom
import { act, createElement, Fragment, type ComponentType, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView, NewWorkflow, Workflow } from "../api";
import i18n from "../i18n";
import WorkflowEditor from "./WorkflowEditor";

const tauri = vi.hoisted(() => ({ invoked: [] as [string, unknown][] }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: { input?: NewWorkflow; workflowId?: string }) => {
    tauri.invoked.push([command, args]);
    const saved = { id: args.workflowId ?? "w-new", ...args.input, created_at: "t", updated_at: "t" };
    return Promise.resolve(saved);
  },
}));

type Props = Record<string, unknown>;
type Item = { id: string; type?: string; selected?: boolean; className?: string; data?: Props } & Props;
vi.mock("@xyflow/react", () => {
  function ReactFlow(props: Props) {
    const nodes = props.nodes as Item[];
    const edges = props.edges as Item[];
    const edgeTypes = (props.edgeTypes ?? {}) as Record<string, ComponentType<Props>>;
    const onNodesChange = props.onNodesChange as ((changes: Props[]) => void) | undefined;
    const onEdgesChange = props.onEdgesChange as ((changes: Props[]) => void) | undefined;
    const select = (id: string, change?: (changes: Props[]) => void) => () =>
      change?.([{ type: "select", id, selected: true }]);
    return createElement(
      "div",
      { className: "flow" },
      ...nodes.map((n) =>
        createElement(
          "button",
          {
            key: n.id,
            "data-node": n.id,
            "aria-pressed": n.selected ? "true" : "false",
            className: n.className,
            onClick: select(n.id, onNodesChange),
          },
          (n.data?.name as string) || n.id,
        ),
      ),
      ...edges.map((e) =>
        createElement(
          "div",
          { key: e.id },
          createElement(
            "button",
            {
              "data-edge": e.id,
              "aria-pressed": e.selected ? "true" : "false",
              className: e.className,
              onClick: select(e.id, onEdgesChange),
            },
            e.id,
          ),
          e.type && edgeTypes[e.type]
            ? createElement(edgeTypes[e.type], {
                ...e,
                selected: !!e.selected,
                sourceX: 0,
                sourceY: 0,
                targetX: 0,
                targetY: 0,
                sourcePosition: "bottom",
                targetPosition: "top",
              })
            : null,
        ),
      ),
    );
  }
  const none = () => null;
  return {
    ReactFlow,
    Handle: none,
    Background: none,
    Controls: none,
    BaseEdge: none,
    EdgeLabelRenderer: ({ children }: { children: ReactNode }) => createElement(Fragment, null, children),
    getSmoothStepPath: () => ["", 0, 0, 0, 0],
    Position: { Left: "left", Right: "right", Top: "top", Bottom: "bottom" },
    MarkerType: { Arrow: "arrow", ArrowClosed: "arrowclosed" },
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = (id: string, handle: string): BotView => ({
  id,
  handle,
  display_name: handle,
  color: "#2563eb",
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status: "idle",
});
const bots = [bot("b-plan", "plan"), bot("b-feature", "feature")];

const arrow = (to: string) => ({ to, outcome: null });
const shipIt: Workflow = {
  id: "w1",
  name: "Ship it",
  when_to_use: "A change to ship.",
  def: {
    version: 1,
    start: "plan",
    steps: [
      { id: "plan", name: "Plan", bot_id: "b-plan", instructions: "Plan it.", arrows: [arrow("build")], pos: null },
      { id: "build", name: "Build", bot_id: "b-feature", instructions: "Build it.", arrows: [arrow("done")], pos: null },
    ],
    start_pos: null,
    done_pos: null,
  },
  created_at: "t",
  updated_at: "t",
};

const handlers = { onSaved: vi.fn(), onCancel: vi.fn() };

let root: Root;
let host: HTMLElement;

async function mount(workflow: Workflow | null, otherNames: string[] = []) {
  await act(async () =>
    root.render(<WorkflowEditor workflow={workflow} bots={bots} otherNames={otherNames} {...handlers} />),
  );
}

const button = (text: string) => {
  const found = [...host.querySelectorAll("button")].find(
    (b) => b.textContent === text || b.getAttribute("aria-label") === text,
  );
  expect(found, text).toBeDefined();
  return found!;
};
const node = (id: string) => host.querySelector<HTMLButtonElement>(`[data-node="${id}"]`);
const edge = (id: string) => host.querySelector<HTMLButtonElement>(`[data-edge="${id}"]`);
const field = <T extends HTMLElement>(label: string) => {
  const found = [...host.querySelectorAll("label")].find((l) => l.textContent?.startsWith(label));
  expect(found, label).toBeDefined();
  return found!.querySelector<T>("input, select, textarea")!;
};
const click = (el: Element) => act(async () => void el.dispatchEvent(new MouseEvent("click", { bubbles: true })));
const type = (el: HTMLInputElement | HTMLTextAreaElement, text: string) =>
  act(async () => {
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
const choose = (el: HTMLSelectElement, value: string) =>
  act(async () => {
    el.value = value;
    el.dispatchEvent(new Event("change", { bubbles: true }));
  });
const press = (key: string, target: EventTarget = document.body) =>
  act(async () => void target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })));
const saved = (command: string) => {
  const call = tauri.invoked.find(([name]) => name === command);
  expect(call, command).toBeDefined();
  return (call![1] as { input: NewWorkflow }).input;
};

describe("the workflow editor", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    for (const handler of Object.values(handlers)) handler.mockReset();
    tauri.invoked = [];
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("+ Step adds a box and selects it", async () => {
    await mount(null);
    expect(host.querySelectorAll("[data-node]")).toHaveLength(2);
    await click(button("+ Step"));
    expect(node("s1")?.textContent).toBe("Step 1");
    expect(node("s1")?.getAttribute("aria-pressed")).toBe("true");
    expect(field<HTMLInputElement>("Step name").value).toBe("Step 1");
    expect(edge("start:0")).not.toBeNull();

    await click(button("+ Step"));
    expect(node("s2")?.getAttribute("aria-pressed")).toBe("true");
    expect(node("s1")?.getAttribute("aria-pressed")).toBe("false");
    expect(field<HTMLInputElement>("Step name").value).toBe("Step 2");
  });

  it("a new step has no bot until one is picked", async () => {
    await mount(null);
    await type(field<HTMLInputElement>("Name"), "Small job");
    await click(button("+ Step"));
    await type(field<HTMLTextAreaElement>("Instructions"), "Do the work.");
    expect(field<HTMLSelectElement>("Bot").value).toBe("");
    expect(host.textContent).toContain("Step Step 1 has no bot.");
    expect(node("s1")?.classList.contains("wf-problem")).toBe(true);
    expect(button("Save").disabled).toBe(true);

    await choose(field<HTMLSelectElement>("Bot"), "b-feature");
    expect(host.textContent).not.toContain("has no bot");
    expect(button("Save").disabled).toBe(false);
  });

  it("editing the side panel changes the saved definition", async () => {
    await mount(shipIt);
    await click(node("plan")!);
    await type(field<HTMLInputElement>("Step name"), "Plan it");
    await choose(field<HTMLSelectElement>("Bot"), "b-feature");
    await type(field<HTMLTextAreaElement>("Instructions"), "Plan it with care.");
    expect(node("plan")?.textContent).toBe("Plan it");
    await click(button("Save"));

    const input = saved("update_workflow");
    expect(tauri.invoked[0][1]).toMatchObject({ workflowId: "w1" });
    expect(input.def.steps[0]).toMatchObject({
      id: "plan",
      name: "Plan it",
      bot_id: "b-feature",
      instructions: "Plan it with care.",
    });
    expect(input.def.start_pos).not.toBeNull();
    expect(input.def.steps.every((s) => s.pos !== null)).toBe(true);
    expect(handlers.onSaved).toHaveBeenCalledTimes(1);
  });

  it("Save is disabled with an empty name", async () => {
    await mount(null);
    await click(button("+ Step"));
    await choose(field<HTMLSelectElement>("Bot"), "b-plan");
    await type(field<HTMLTextAreaElement>("Instructions"), "Do the work.");
    expect(button("Save").disabled).toBe(true);
    expect(host.textContent).toContain("Give the workflow a name.");

    await type(field<HTMLInputElement>("Name"), "Small job");
    expect(button("Save").disabled).toBe(false);
    expect(host.textContent).not.toContain("Give the workflow a name.");
    await type(field<HTMLInputElement>("Name"), "   ");
    expect(button("Save").disabled).toBe(true);
  });

  it("Save calls api.createWorkflow with the definition, then onSaved", async () => {
    await mount(null);
    await type(field<HTMLInputElement>("Name"), "  Small job ");
    await type(field<HTMLInputElement>("When to use"), "A quick change.");
    await click(button("+ Step"));
    await choose(field<HTMLSelectElement>("Bot"), "b-plan");
    await type(field<HTMLTextAreaElement>("Instructions"), "Do the work.");
    await click(button("Save"));

    expect(tauri.invoked.map(([name]) => name)).toEqual(["create_workflow"]);
    const input = saved("create_workflow");
    expect(input.name).toBe("Small job");
    expect(input.when_to_use).toBe("A quick change.");
    expect(input.def.start).toBe("s1");
    expect(input.def.steps).toEqual([
      {
        id: "s1",
        name: "Step 1",
        bot_id: "b-plan",
        instructions: "Do the work.",
        arrows: [],
        pos: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
      },
    ]);
    const first = input.def.steps[0].pos!;
    expect(first.y).toBeGreaterThan(input.def.start_pos!.y + 48);
    expect(input.def.done_pos!.y).toBeGreaterThan(first.y + 64);
    expect(handlers.onSaved).toHaveBeenCalledWith(expect.objectContaining({ id: "w-new", name: "Small job" }));
  });

  it("lists the problems and outlines the boxes and arrows they concern", async () => {
    await mount(shipIt, ["ship IT"]);
    expect(button("Save").disabled).toBe(true);
    expect(host.textContent).toContain("There is already a workflow named Ship it.");

    await click(node("build")!);
    await type(field<HTMLTextAreaElement>("Instructions"), " ");
    expect(host.textContent).toContain("Write instructions for step Build.");
    expect(node("build")?.classList.contains("wf-problem")).toBe(true);
    expect(node("plan")?.classList.contains("wf-problem")).toBe(false);
    expect(edge("plan:0")?.classList.contains("wf-problem")).toBe(false);
  });

  it("outlines the arrows of a step with several arrows until each is named", async () => {
    const [plan, build] = shipIt.def.steps;
    const forked = { ...plan, arrows: [arrow("build"), arrow("done")] };
    await mount({ ...shipIt, def: { ...shipIt.def, steps: [forked, build] } });
    expect(host.textContent).toContain("When step Plan has several arrows, name each one.");
    expect(edge("plan:0")?.classList.contains("wf-problem")).toBe(true);
    expect(edge("plan:1")?.classList.contains("wf-problem")).toBe(true);
    expect(edge("build:0")?.classList.contains("wf-problem")).toBe(false);

    await click(edge("plan:0")!);
    await type(host.querySelector<HTMLInputElement>(".wf-edge-label input")!, "build it");
    await click(edge("plan:1")!);
    await type(host.querySelector<HTMLInputElement>(".wf-edge-label input")!, "nothing to do");
    expect(edge("plan:0")?.classList.contains("wf-problem")).toBe(false);
    expect(edge("plan:1")?.classList.contains("wf-problem")).toBe(false);
    expect(button("Save").disabled).toBe(false);
  });

  it("a selected arrow takes its outcome inline", async () => {
    await mount(shipIt);
    expect(host.querySelector(".wf-edge-label input")).toBeNull();
    await click(edge("plan:0")!);
    const outcome = host.querySelector<HTMLInputElement>(".wf-edge-label input")!;
    expect(outcome).not.toBeNull();
    await type(outcome, "planned");
    await click(button("Save"));
    expect(saved("update_workflow").def.steps[0].arrows).toEqual([{ to: "build", outcome: "planned" }]);
  });

  it("Delete or Backspace removes the selection, but not while typing", async () => {
    await mount(shipIt);
    await click(node("build")!);
    await press("Backspace", field<HTMLInputElement>("Step name"));
    expect(node("build")).not.toBeNull();
    await press("Delete");
    expect(node("build")).toBeNull();
    expect(edge("plan:0")).toBeNull();

    await click(node("start")!);
    await press("Backspace");
    expect(node("start")).not.toBeNull();

    await click(edge("start:0")!);
    await press("Backspace");
    expect(edge("start:0")).toBeNull();
    expect(host.textContent).toContain("Choose which step the workflow starts with.");
  });
});
