// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../i18n";
import { projectApi, type BotView, type Project } from "../api";
import { ProjectApiContext } from "../projectContext";
import TaskForm from "./TaskForm";
import ProjectForm from "./ProjectForm";
import { ActionMenu } from "./ui";

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), ask: vi.fn(), open: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: bridge.ask, open: bridge.open }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const bot = { id: "bot", handle: "helper", display_name: "Helper" } as BotView;
const existing = { id: "project", name: "Research", default_directory: "/research", default_provider: "claude" } as Project;
let host: HTMLElement, root: Root;
const saved = vi.fn(), closed = vi.fn();
const deferred = <T,>() => { let resolve!: (value: T) => void, reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
function Harness({ kind = "task", project }: { kind?: "task" | "project"; project?: Project }) {
  const [open, setOpen] = useState(false);
  const close = () => { closed(); setOpen(false); };
  return <ProjectApiContext.Provider value={projectApi("selected")}><button onClick={() => setOpen(true)}>Open editor</button>
    {open && (kind === "task" ? <TaskForm bots={[bot]} onSaved={saved} onCancel={close} /> : <ProjectForm project={project} onSaved={saved} onClose={close} />)}
  </ProjectApiContext.Provider>;
}
const button = (name: string) => [...host.querySelectorAll("button")].find((element) => element.textContent === name || element.getAttribute("aria-label") === name)!;
const click = (element: HTMLElement) => act(async () => element.click());
const form = () => host.querySelector("form")!;
const cancel = () => act(async () => host.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
const submit = () => act(async () => form().dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
async function fill(selector: string, value: string) {
  const input = host.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function mount(kind: "task" | "project" = "task", project?: Project) {
  await act(async () => root.render(<Harness kind={kind} project={project} />));
  button("Open editor").focus(); await click(button("Open editor"));
}
beforeEach(() => {
  bridge.invoke.mockReset().mockResolvedValue({ id: "saved" }); bridge.ask.mockReset().mockResolvedValue(true); bridge.open.mockReset().mockResolvedValue("/chosen");
  saved.mockReset(); closed.mockReset(); host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

describe("conversation action menu", () => {
  it("keeps actions reachable, restores focus on Escape, and closes after choosing an action or leaving the menu", async () => {
    const action = vi.fn();
    await act(async () => root.render(<><ActionMenu label="Conversation actions"><button onClick={action}>Edit bot</button></ActionMenu><button>Outside</button></>));
    const details = host.querySelector("details")!, summary = host.querySelector("summary")!;
    details.open = true; button("Edit bot").focus();
    await act(async () => button("Edit bot").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(details.open).toBe(false); expect(document.activeElement).toBe(summary); expect(action).not.toHaveBeenCalled();
    details.open = true; await click(button("Edit bot"));
    expect(action).toHaveBeenCalledOnce(); expect(details.open).toBe(false); expect(document.activeElement).toBe(summary);
    details.open = true; button("Edit bot").focus(); button("Outside").focus();
    expect(details.open).toBe(false);
  });
});

describe("shared editor dialogs", () => {
  it("focuses the name, ignores backdrop clicks, and restores the opener after closing an empty form", async () => {
    await mount();
    expect(host.querySelector("dialog")?.open).toBe(true);
    expect(document.activeElement).toBe(host.querySelector("input"));
    await click(host.querySelector("dialog")!); expect(closed).not.toHaveBeenCalled();
    await cancel();
    expect(bridge.ask).not.toHaveBeenCalled(); expect(closed).toHaveBeenCalledOnce();
    expect(document.activeElement).toBe(button("Open editor"));
  });
  it("validates recurring task fields without sending an incomplete schedule", async () => {
    await mount(); await submit();
    expect(host.querySelectorAll('[role="alert"]').length).toBeGreaterThan(0);
    expect(bridge.invoke).not.toHaveBeenCalled();
  });
  it("keeps a draft when discard is cancelled and blocks submission while confirmation is pending", async () => {
    await mount(); await fill("input", "Weekly review"); await fill("textarea", "Review recent work");
    const confirmation = deferred<boolean>(); bridge.ask.mockReturnValueOnce(confirmation.promise);
    await cancel(); await submit(); await cancel();
    expect(bridge.ask).toHaveBeenCalledOnce(); expect(bridge.invoke).not.toHaveBeenCalled();
    await act(async () => confirmation.resolve(false));
    expect(host.querySelector("input")?.value).toBe("Weekly review"); expect(closed).not.toHaveBeenCalled();
    await cancel(); expect(closed).toHaveBeenCalledOnce();
  });
  it("submits a recurring task once, preserves failed drafts, and permits retry", async () => {
    await mount(); await fill("input", "Weekly review"); await fill("textarea", "Review recent work");
    const request = deferred<unknown>(); bridge.invoke.mockReturnValueOnce(request.promise);
    await submit(); await submit(); await cancel();
    expect(bridge.invoke).toHaveBeenCalledOnce(); expect(closed).not.toHaveBeenCalled();
    expect(bridge.invoke.mock.calls[0]).toMatchObject(["create_task", { projectId: "selected", input: { name: "Weekly review", bot_id: "bot", prompt: "Review recent work", every_days: 7, at: "09:00" } }]);
    await act(async () => request.reject(new Error("Save failed")));
    expect(host.querySelector("textarea")?.value).toBe("Review recent work"); expect(host.querySelector('[role="alert"]')).not.toBeNull();
    await submit(); expect(saved).toHaveBeenCalledOnce();
  });
  it("does not navigate from a late schedule save after unmounting", async () => {
    await mount(); await fill("input", "Weekly review"); await fill("textarea", "Review recent work");
    const request = deferred<unknown>(); bridge.invoke.mockReturnValueOnce(request.promise); await submit();
    await act(async () => root.render(<div>Different project</div>));
    await act(async () => request.resolve({ id: "late" })); expect(saved).not.toHaveBeenCalled();
  });
  it("requires a directory for new projects and saves the chosen directory", async () => {
    await mount("project"); await fill("input", "Research"); await submit(); expect(bridge.invoke).not.toHaveBeenCalled();
    await click(button("Choose…")); await submit();
    expect(bridge.invoke).toHaveBeenCalledWith("create_project", { input: { name: "Research", default_directory: "/chosen", default_provider: "claude" } });
    expect(saved).toHaveBeenCalledOnce();
  });
  it("preserves project fields when updating fails, without changing the directory requirement for existing projects", async () => {
    await mount("project", { ...existing, default_directory: null }); await fill("input", "Updated research");
    bridge.invoke.mockRejectedValueOnce(new Error("Update failed")); await submit();
    expect(bridge.invoke).toHaveBeenCalledWith("update_project", { projectId: "project", input: { name: "Updated research", default_directory: null, default_provider: "claude" } });
    expect(host.querySelector("input")?.value).toBe("Updated research"); expect(host.querySelector('[role="alert"]')).not.toBeNull();
    await submit(); expect(saved).toHaveBeenCalledOnce();
  });
  it("ignores a directory picker result after the project form is replaced", async () => {
    await mount("project"); const picker = deferred<string>(); bridge.open.mockReturnValueOnce(picker.promise);
    await click(button("Choose…")); await cancel(); expect(closed).not.toHaveBeenCalled();
    await act(async () => root.render(<div>Different project</div>));
    await act(async () => picker.resolve("/late")); expect(host.textContent).toBe("Different project"); expect(saved).not.toHaveBeenCalled();
  });
});
