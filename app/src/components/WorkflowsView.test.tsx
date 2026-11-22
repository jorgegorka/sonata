// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView, Workflow, WorkflowStep } from "../api";
import i18n from "../i18n";
import WorkflowsView from "./WorkflowsView";

const tauri = vi.hoisted(() => ({
  asked: [] as string[],
  answer: true,
  invoked: [] as [string, unknown][],
  result: null as unknown,
  failure: null as unknown,
  picked: null as string | null,
  saved: null as string | null,
  saveOptions: null as unknown,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: () => Promise.resolve(tauri.picked),
  save: (options: unknown) => {
    tauri.saveOptions = options;
    return Promise.resolve(tauri.saved);
  },
  ask: (message: string) => {
    tauri.asked.push(message);
    return Promise.resolve(tauri.answer);
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    tauri.invoked.push([command, args]);
    return tauri.failure ? Promise.reject(tauri.failure) : Promise.resolve(tauri.result);
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = (id: string, handle: string): BotView => ({ project_id: "default",
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
const bots = [bot("b-plan", "plan"), bot("b-feature", "feature"), bot("b-review", "review")];

const step = (id: string, name: string, bot_id: string, to: string): WorkflowStep => ({
  id,
  name,
  bot_id,
  instructions: "Do it.",
  arrows: [{ to, outcome: null }],
  pos: null,
});
const workflow = (id: string, name: string, when_to_use: string, steps: WorkflowStep[]): Workflow => ({ project_id: "default",
  id,
  name,
  when_to_use,
  def: { version: 1, start: steps[0]?.id ?? null, steps, start_pos: null, done_pos: null },
  created_at: "t",
  updated_at: "t",
});
const ticket = workflow("w1", "Ticket to merge request", "A request to build.", [
  step("plan", "Plan", "b-plan", "implement"),
  step("implement", "Implement", "b-feature", "review"),
  step("review", "Review", "b-review", "done"),
]);
const quick = workflow("w2", "Quick fix", "", [step("fix", "Fix", "b-gone", "done")]);

const handlers = { onImported: vi.fn(), onNew: vi.fn(), onEdit: vi.fn(), onRun: vi.fn(), onRequested: vi.fn() };

let root: Root;
let host: HTMLElement;

async function mount(workflows: Workflow[]) {
  await act(async () => root.render(<WorkflowsView workflows={workflows} bots={bots} {...handlers} />));
}

const rows = () => [...host.querySelectorAll("li")];
const buttonIn = (el: Element, text: string) => {
  const found = [...el.querySelectorAll("button")].find((b) => b.textContent === text);
  expect(found, text).toBeDefined();
  return found!;
};
const click = (el: Element) => act(async () => void el.dispatchEvent(new MouseEvent("click", { bubbles: true })));
const box = () => host.querySelector("textarea")!;
async function type(text: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(box(), text);
    box().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function press(key: string, metaKey = false) {
  await act(async () => {
    box().dispatchEvent(new KeyboardEvent("keydown", { key, metaKey, bubbles: true, cancelable: true }));
  });
}
const sendButton = () => buttonIn(host, "Create with Master");

describe("the Workflows page", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    for (const handler of Object.values(handlers)) handler.mockReset();
    tauri.asked = [];
    tauri.invoked = [];
    tauri.answer = true;
    tauri.result = null;
    tauri.failure = null;
    tauri.picked = null;
    tauri.saved = null;
    tauri.saveOptions = null;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("it lists the rows", async () => {
    await mount([ticket, quick]);
    const [first, second] = rows();
    expect(rows()).toHaveLength(2);
    expect(first.textContent).toContain("Ticket to merge request");
    expect(first.textContent).toContain("A request to build.");
    expect(first.textContent).toContain("Plan → Implement → Review");
    expect(second.textContent).toContain("Quick fix");
    expect(second.textContent).toContain("Step Fix has no bot.");
    expect(first.textContent).not.toContain("has no bot");

    await click(buttonIn(first, "Run"));
    expect(handlers.onRun).toHaveBeenCalledWith("w1");
    await click(buttonIn(second, "Edit"));
    expect(handlers.onEdit).toHaveBeenCalledWith("w2");
    await click(buttonIn(host, "New manual workflow"));
    expect(handlers.onNew).toHaveBeenCalledTimes(1);
  });

  it("sends a description to Master below the list, and follows the session it opened", async () => {
    await mount([ticket]);
    const list = host.querySelector("ul")!;
    const form = host.querySelector("form.workflow-request")!;
    expect(list.compareDocumentPosition(form) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(form.textContent).toContain("Describe a new workflow");
    expect(sendButton().disabled).toBe(true);
    await type("   ");
    expect(sendButton().disabled, "only spaces").toBe(true);

    const sent = { session: { id: "s1" }, message: { id: "m1" } };
    tauri.result = sent;
    await type("An SEO workflow\nwith tickets");
    expect(sendButton().disabled).toBe(false);
    await click(sendButton());
    expect(tauri.invoked).toEqual([["request_workflow", { body: "An SEO workflow\nwith tickets" }]]);
    expect(handlers.onRequested).toHaveBeenCalledWith(sent);
    expect(box().value).toBe("");
  });

  it("⌘Enter sends the description; Enter alone does not", async () => {
    await mount([]);
    tauri.result = { session: { id: "s1" }, message: { id: "m1" } };
    await type("An SEO workflow");
    await press("Enter");
    expect(tauri.invoked).toEqual([]);
    await press("Enter", true);
    expect(tauri.invoked).toEqual([["request_workflow", { body: "An SEO workflow" }]]);
  });

  it("keeps the description and shows why when Master is gone", async () => {
    await mount([]);
    tauri.failure = { code: "masterMissing", params: {} };
    await type("An SEO workflow");
    await click(sendButton());
    expect(handlers.onRequested).not.toHaveBeenCalled();
    expect(box().value).toBe("An SEO workflow");
    expect(host.querySelector('form.workflow-request [role="alert"]')?.textContent).toContain(
      "Master is no longer here",
    );
  });

  it("says when there are no workflows", async () => {
    await mount([]);
    expect(rows()).toHaveLength(0);
    expect(host.textContent).toContain("No workflows yet.");
  });

  it("exports the selected workflow with a safe filename and reports the dependency counts", async () => {
    await mount([ticket]);
    tauri.saved = "/tmp/workflow.yaml";
    tauri.result = { workflows: 1, bots: 3, skills: 4 };
    await click(buttonIn(rows()[0], "Export…"));
    expect(tauri.invoked).toEqual([["export_workflow_config", { workflowId: "w1", path: "/tmp/workflow.yaml" }]]);
    expect(tauri.saveOptions).toEqual(
      expect.objectContaining({ defaultPath: expect.stringContaining("sonata-workflow-Ticket to merge request-") }),
    );
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Bots: 3");
  });

  it("exports the full configuration from the toolbar and reports all saved items", async () => {
    await mount([ticket, quick]);
    tauri.saved = "/tmp/everything.yaml";
    tauri.result = { workflows: 2, bots: 5, skills: 7 };
    const exportAll = buttonIn(host, "Export project…");
    const hint = document.getElementById(exportAll.getAttribute("aria-describedby")!);
    expect(hint?.textContent).toContain("this project’s workflows, bots and referenced skills, including skill files");
    expect(hint?.textContent).toContain("Settings → Configuration → Import");
    await click(exportAll);
    expect(tauri.invoked).toEqual([["export_config", { path: "/tmp/everything.yaml" }]]);
    expect(tauri.saveOptions).toEqual({
      defaultPath: expect.stringMatching(/^sonata-config-\d{4}-\d{2}-\d{2}\.yaml$/),
      filters: [{ name: "Sonata configuration", extensions: ["yaml", "yml"] }],
    });
    expect(host.querySelector('[role="status"]')?.textContent).toBe(
      "Saved everything.yaml. Bots: 5 · Skills: 7 · Workflows: 2",
    );
  });

  it("can export bots and skills even when there are no workflows", async () => {
    await mount([]);
    tauri.saved = "/tmp/team.yaml";
    tauri.result = { workflows: 0, bots: 3, skills: 2 };
    expect(buttonIn(host, "Export project…").disabled).toBe(false);
    await click(buttonIn(host, "Export project…"));
    expect(tauri.invoked).toEqual([["export_config", { path: "/tmp/team.yaml" }]]);
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Bots: 3 · Skills: 2 · Workflows: 0");
  });

  it("cancels a full export without writing a file", async () => {
    await mount([ticket]);
    await click(buttonIn(host, "Export project…"));
    expect(tauri.invoked).toEqual([]);
    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(buttonIn(host, "Export project…").disabled).toBe(false);
  });

  it("explains a failed full export and clears the previous success message", async () => {
    await mount([ticket]);
    tauri.saved = "/tmp/team.yaml";
    tauri.result = { workflows: 1, bots: 3, skills: 2 };
    await click(buttonIn(host, "Export project…"));
    expect(host.querySelector('[role="status"]')).not.toBeNull();
    tauri.failure = { code: "configExportTooLarge", params: { mb: "256" } };
    await click(buttonIn(host, "Export project…"));
    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("more than 256 MB");
    expect(buttonIn(host, "Export project…").disabled).toBe(false);
  });

  it("prevents overlapping imports and exports while the full file is being saved", async () => {
    await mount([ticket]);
    tauri.saved = "/tmp/team.yaml";
    let finish!: (value: unknown) => void;
    tauri.result = new Promise((resolve) => { finish = resolve; });
    await click(buttonIn(host, "Export project…"));
    expect(buttonIn(host, "Export project…").disabled).toBe(true);
    expect(buttonIn(host, "Import workflow…").disabled).toBe(true);
    expect(buttonIn(rows()[0], "Export…").disabled).toBe(true);
    await act(async () => finish({ workflows: 1, bots: 3, skills: 2 }));
    expect(buttonIn(host, "Export project…").disabled).toBe(false);
    expect(buttonIn(host, "Import workflow…").disabled).toBe(false);
    expect(buttonIn(rows()[0], "Export…").disabled).toBe(false);
  });

  it("cancels file selection without API calls and explains files without workflows", async () => {
    await mount([]);
    await click(buttonIn(host, "Import workflow…"));
    expect(tauri.invoked).toEqual([]);
    tauri.picked = "/tmp/skills.yaml";
    tauri.result = [];
    await click(buttonIn(host, "Import workflow…"));
    expect(tauri.invoked).toEqual([["workflows_in_config", { path: "/tmp/skills.yaml" }]]);
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("contains no workflows");
  });

  it("Delete asks first and calls api.deleteWorkflow only on yes", async () => {
    await mount([ticket]);
    tauri.answer = false;
    await click(buttonIn(rows()[0], "Delete"));
    expect(tauri.asked).toEqual(["Delete Ticket to merge request? Its runs' sessions stay on the board."]);
    expect(tauri.invoked).toEqual([]);

    tauri.answer = true;
    await click(buttonIn(rows()[0], "Delete"));
    expect(tauri.invoked).toEqual([["delete_workflow", { workflowId: "w1" }]]);
  });
});
