// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView, NewWorkItem, UiEvent, WorkItem } from "./api";
import App from "./App";

const bridge = vi.hoisted(() => ({
  calls: [] as [string, unknown][],
  answers: {} as Record<string, (args: any) => unknown>,
  listeners: new Set<(event: { payload: UiEvent }) => void>(),
  picked: ["/tmp/report.txt"],
  ask: vi.fn<(...args: unknown[]) => Promise<boolean>>(),
  drop: null as ((event: { payload: { type: string; paths: string[] } }) => void) | null,
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: async (name: string, args: unknown) => {
  bridge.calls.push([name, args]); return bridge.answers[name]?.(args) ?? null;
} }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, listener: (e: { payload: UiEvent }) => void) => {
  if (name === "ui-event") bridge.listeners.add(listener);
  return () => bridge.listeners.delete(listener);
} }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: bridge.ask, open: async () => bridge.picked }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async (listener: typeof bridge.drop) => {
  bridge.drop = listener; return () => { if (bridge.drop === listener) bridge.drop = null; };
} }) }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = (id: string, project_id = "default"): BotView => ({ id, project_id, handle: id, display_name: id,
  color: "#456789", provider: "claude", model: null, instructions: "", config_version: 1, skill_ids: [], created_at: "t", updated_at: "t", status: "offline" });
const item = (id: string, project_id = "default"): WorkItem => ({ id, project_id, parent_id: null, title: id, instructions: "Work", state: "pending",
  outcome: null, primary_session_id: null, session_ids: [], workflow_run_id: null, orchestration_id: null, execution: null, manual_start: true,
  created_at: "2026-11-27T12:00:00Z", finished_at: null, archived_at: null, result_message_id: null, depends_on: [] });
const workflow = { id: "flow", project_id: "default", name: "Review", when_to_use: "", def: { version: 1, start: "first", steps: [
  { id: "first", name: "Review", bot_id: "worker", instructions: "Review", arrows: [], automatic_child_review: false },
], start_pos: null, done_pos: null }, created_at: "t", updated_at: "t" };
let root: Root, host: HTMLElement, items: WorkItem[];
const calls = (name: string) => bridge.calls.filter(([command]) => command === name).map(([, args]) => args);
const form = () => host.querySelector<HTMLFormElement>(".work-form")!;
const dialog = () => host.querySelector<HTMLDialogElement>("dialog.task-dialog")!;
const cancel = () => act(async () => dialog().dispatchEvent(new Event("cancel", { cancelable: true })));
const key = (value: string, modifiers: KeyboardEventInit = {}) => act(async () => {
  form().querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, ...modifiers }));
});
function button(text: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === text || b.getAttribute("aria-label") === text);
  expect(found, text).toBeDefined(); return found!;
}
const click = (element: HTMLElement) => act(async () => element.click());
const mount = () => act(async () => root.render(<App />));
async function fill(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function project(id: string) {
  await act(async () => { const select = host.querySelector<HTMLSelectElement>('select[aria-label="Project"]')!;
    select.value = id; select.dispatchEvent(new Event("change", { bubbles: true })); });
}
const refresh = () => act(async () => { bridge.listeners.forEach((fn) => fn({ payload: { type: "work_items_changed" } })); });
const deferred = <T,>() => { let resolve!: (value: T) => void, reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

beforeEach(() => {
  globalThis.localStorage?.clear(); items = []; bridge.calls = []; bridge.listeners.clear(); bridge.picked = ["/tmp/report.txt"];
  bridge.ask.mockReset().mockResolvedValue(true);
  bridge.answers = {
    get_settings: () => ({ setup_done: true, language_resolved: "en" }),
    list_projects: () => ["default", "second"].map((id) => ({ id, name: id, default_directory: null, default_provider: "claude", archived_at: null })),
    list_bots: () => [bot("master"), bot("worker"), bot("second-worker", "second")],
    list_sessions: () => [], list_bot_threads: () => [], get_paused: () => false, usage_limits: () => [], detect_providers: () => [],
    list_skills: () => [], list_tasks: () => [], list_workflows: () => [workflow], session_activity: () => [], session_glances: () => [],
    list_work_items: ({ projectId }: { projectId: string }) => items.filter((w) => w.project_id === projectId),
    create_work_item: ({ input, projectId }: { input: NewWorkItem; projectId: string }) => {
      const created = { ...item("created", projectId), parent_id: input.parent_id, title: input.title || input.instructions };
      items.push(created); return created;
    },
    create_bot: () => bot("new-bot"),
  };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

describe("one task creation interface", () => {
  it("uses the shared form from the board, bot shortcut, workflow Run, and a newly created bot", async () => {
    await mount(); await click(button("New task"));
    expect(dialog().open).toBe(true);
    expect(host.querySelector("aside.drawer")).toBeNull();
    expect(document.activeElement).toBe(form().querySelector("textarea"));
    expect(form().textContent).toContain("Title (optional)");
    expect(form().querySelector("select")?.value).toBe("bot:master");
    await click(button("Close"));
    await click(host.querySelector<HTMLButtonElement>('button[aria-label^="worker ·"]')!);
    expect(dialog().open).toBe(true);
    expect(form().querySelector("select")?.value).toBe("bot:worker");
    await click(button("Close"));
    await click(button("Workflows")); await click(button("Run"));
    expect(dialog().open).toBe(true);
    expect(form().querySelector("select")?.value).toBe("workflow:flow");
    expect(host.querySelector(".workflow-start-map")).toBeNull();
    await fill(form().querySelector("textarea")!, "Review the change"); await click(button("Create task"));
    expect(calls("create_work_item")).toEqual([{ projectId: "default", input: {
      parent_id: null, title: "", instructions: "Review the change", executor: { kind: "workflow", name: "Review" }, manual_start: false, depends_on: [], files: [],
    } }]);
    await click(button("New bot"));
    await fill(host.querySelector<HTMLInputElement>('form.create-bot input')!, "new-bot");
    await act(async () => host.querySelector("form.create-bot")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(form().querySelector("select")?.value).toBe("bot:new-bot");
    expect(calls("send_message")).toEqual([]); expect(calls("start_workflow")).toEqual([]);
  });

  it("opens the same form for subtasks with sibling dependencies", async () => {
    items = [item("parent"), { ...item("sibling"), parent_id: "parent" }];
    await mount(); await click(button("Show Pending (2)"));
    await click(button("Add subtask"));
    expect(dialog().open).toBe(true);
    expect(form().textContent).toContain("Parent: parent");
    expect(form().querySelector("fieldset")).toBeNull();
    await click(button("Depends on"));
    expect(form().querySelector("fieldset")?.textContent).toContain("sibling");
    await fill(form().querySelector("textarea")!, "Child instructions");
    await click(form().querySelector<HTMLInputElement>('fieldset input')!);
    await click(button("Create task"));
    expect(calls("create_work_item")[0]).toMatchObject({ input: { parent_id: "parent", depends_on: ["sibling"] } });
  });

  it("keeps drafts and attachments through delayed loading, a failure, and retry", async () => {
    const read = deferred<WorkItem[]>(); bridge.answers.list_work_items = () => read.promise;
    await mount(); expect(host.textContent).toContain("Loading tasks");
    await click(button("New task")); await fill(form().querySelector("textarea")!, "Keep this draft");
    await click(button("Attach files"));
    await act(async () => bridge.drop?.({ payload: { type: "drop", paths: ["/tmp/report.txt", "/tmp/image.png"] } }));
    expect(form().querySelectorAll(".chip")).toHaveLength(2);
    await act(async () => read.reject(new Error("Task load failed")));
    expect(host.textContent).toContain("Could not load tasks.");
    expect(form().querySelector("textarea")?.value).toBe("Keep this draft");
    bridge.answers.list_work_items = () => [];
    await click(button("Retry"));
    expect(form().querySelectorAll(".chip")).toHaveLength(2);
    await click(button("Create task"));
    expect(calls("create_work_item")[0]).toMatchObject({ input: { files: ["/tmp/report.txt", "/tmp/image.png"] } });
    expect(form()).toBeNull();
  });

  it("preserves loaded cards on refresh failure and allows creation while the first read is pending", async () => {
    items = [item("existing")]; await mount(); await click(button("Show Pending (1)"));
    bridge.answers.list_work_items = () => Promise.reject(new Error("Refresh failed")); await refresh();
    expect(host.querySelector(".task-title")?.textContent).toBe("existing");
    await click(button("New task")); await fill(form().querySelector("textarea")!, "Independent creation");
    await click(button("Create task")); expect(calls("create_work_item")).toHaveLength(1);
  });

  it("preserves form contents after a failed create and prevents duplicate submissions", async () => {
    const pending = deferred<WorkItem>(); bridge.answers.create_work_item = () => pending.promise;
    await mount(); await click(button("New task")); await fill(form().querySelector("textarea")!, "Do this");
    await click(button("Attach files"));
    const submit = button("Create task");
    await click(submit); await click(submit);
    expect(submit.disabled).toBe(true);
    expect(button("Close").disabled).toBe(true);
    await cancel(); await key("Enter", { metaKey: true });
    expect(dialog().open).toBe(true);
    expect(bridge.ask).not.toHaveBeenCalled();
    expect(calls("create_work_item")).toHaveLength(1);
    await act(async () => pending.reject(new Error("File unavailable")));
    expect(form().textContent).toContain("File unavailable");
    expect(form().querySelector("textarea")?.value).toBe("Do this");
    expect(form().querySelectorAll(".chip")).toHaveLength(1);
    await click(button("Remove report.txt")); expect(form().querySelectorAll(".chip")).toHaveLength(0);
  });

  it("closes an empty dialog and restores focus without asking", async () => {
    await mount(); const trigger = button("New task"); trigger.focus();
    await click(trigger); await cancel();
    expect(dialog()).toBeNull(); expect(document.activeElement).toBe(trigger);
    expect(bridge.ask).not.toHaveBeenCalled();
  });

  it("ignores backdrop clicks and preserves a draft when discard is cancelled", async () => {
    await mount(); await click(button("New task"));
    await fill(form().querySelector("textarea")!, "Keep my description");
    await click(button("Attach files"));
    await click(dialog()); expect(dialog().open).toBe(true);
    expect(bridge.ask).not.toHaveBeenCalled();
    bridge.ask.mockResolvedValueOnce(false);
    await cancel();
    expect(form().querySelector("textarea")?.value).toBe("Keep my description");
    expect(form().querySelectorAll(".chip")).toHaveLength(1);
    expect(bridge.ask).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ okLabel: "Discard draft", cancelLabel: "Keep editing" }));
    await click(button("Close")); expect(dialog()).toBeNull();
    expect(calls("create_work_item")).toEqual([]);
  });

  it("collapses properties before Escape can discard changed start settings", async () => {
    await mount(); await click(button("New task"));
    const trigger = button("Start automatically");
    await click(trigger);
    await click(form().querySelectorAll<HTMLInputElement>('input[type="radio"]')[1]);
    await key("Escape");
    expect(form().querySelector("fieldset")).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(bridge.ask).not.toHaveBeenCalled();
    expect(trigger.textContent).toBe("Hold for manual start");
    bridge.ask.mockResolvedValueOnce(false);
    await cancel(); expect(bridge.ask).toHaveBeenCalledOnce(); expect(dialog().open).toBe(true);
  });

  it.each(["title", "attachment", "executor"])("confirms discarding a draft changed only by %s", async (changed) => {
    await mount(); await click(button("New task"));
    if (changed === "title") await fill(form().querySelector("input")!, "Draft title");
    if (changed === "attachment") await click(button("Attach files"));
    if (changed === "executor") await act(async () => {
      const select = form().querySelector("select")!; select.value = "bot:worker";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await click(button("Close")); expect(bridge.ask).toHaveBeenCalledOnce(); expect(dialog()).toBeNull();
  });

  it("does not submit, attach, or open another confirmation while discarding", async () => {
    const answer = deferred<boolean>(); bridge.ask.mockReturnValue(answer.promise);
    await mount(); await click(button("New task")); await fill(form().querySelector("textarea")!, "Draft");
    await cancel(); await cancel(); await key("Enter", { ctrlKey: true });
    await act(async () => bridge.drop?.({ payload: { type: "drop", paths: ["/tmp/late.txt"] } }));
    expect(bridge.ask).toHaveBeenCalledOnce(); expect(calls("create_work_item")).toEqual([]);
    expect(form().querySelectorAll(".chip")).toHaveLength(0);
    await act(async () => answer.resolve(false));
    expect(button("Create task").disabled).toBe(false);
  });

  it.each(["metaKey", "ctrlKey"])("submits valid descriptions with %s+Enter, while ordinary Enter stays in the editor", async (modifier) => {
    await mount(); await click(button("New task"));
    await key("Enter", { [modifier]: true }); expect(calls("create_work_item")).toEqual([]);
    await fill(form().querySelector("textarea")!, "Describe the work");
    await key("Enter"); expect(calls("create_work_item")).toEqual([]);
    await key("Enter", { [modifier]: true });
    expect(calls("create_work_item")).toHaveLength(1); expect(dialog()).toBeNull();
  });

  it("can retry workflow loading and prevents starting a workflow with a missing bot", async () => {
    bridge.answers.list_workflows = () => Promise.reject(new Error("Workflow load failed"));
    await mount(); await click(button("New task"));
    expect(form().textContent).toContain("Workflow load failed");
    bridge.answers.list_workflows = () => [{ ...workflow, def: { ...workflow.def, steps: [{ ...workflow.def.steps[0], bot_id: "missing" }] } }];
    await click(button("Retry"));
    await act(async () => { const select = form().querySelector("select")!; select.value = "workflow:flow";
      select.dispatchEvent(new Event("change", { bubbles: true })); });
    await fill(form().querySelector("textarea")!, "Work");
    expect(button("Create task").disabled).toBe(true);
    expect(form().querySelector('[role="alert"]')).not.toBeNull();
  });

  it("does not let late reads or creates close a different project's form", async () => {
    const read = deferred<WorkItem[]>(), create = deferred<WorkItem>();
    bridge.answers.list_work_items = ({ projectId }) => projectId === "default" ? read.promise : [];
    bridge.answers.create_work_item = () => create.promise;
    await mount(); await click(button("New task")); await fill(form().querySelector("textarea")!, "First project");
    await click(button("Create task"));
    await project("second"); expect(form()).toBeNull(); await click(button("New task"));
    await fill(form().querySelector("textarea")!, "Second project");
    await act(async () => { read.resolve([item("old")]); create.resolve(item("late")); });
    expect(form().querySelector("textarea")?.value).toBe("Second project");
    expect(host.querySelectorAll(".task-title")).toHaveLength(0);
  });
});
