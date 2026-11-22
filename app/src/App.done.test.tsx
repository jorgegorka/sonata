// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView, Message, SessionView, ThreadSummary, UiEvent } from "./api";
import App from "./App";

const tauri = vi.hoisted(() => ({
  calls: [] as [string, unknown][],
  listeners: new Set<(e: { payload: UiEvent }) => void>(),
  answers: {} as Record<string, (args: unknown) => unknown>,
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (command: string, args: unknown) => {
    tauri.calls.push([command, args]);
    return tauri.answers[command]?.(args) ?? null;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (event: string, handler: (e: { payload: UiEvent }) => void) => {
    if (event === "ui-event") tauri.listeners.add(handler);
    return () => tauri.listeners.delete(handler);
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: async () => true }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date().toISOString();
const OLD = "2026-01-01T00:00:00.000Z";
const bot = (id: string): BotView => ({
  id, handle: id, display_name: id, color: "#4f46e5", provider: "claude", model: null,
  instructions: "", config_version: 1, skill_ids: [], created_at: OLD, updated_at: OLD, status: "offline",
});
const session = (id: string, extra: Partial<SessionView> = {}): SessionView => ({
  id, bot_id: "master", number: 1, title: id, provider_session_id: null, session_config_version: null,
  opened_by: null, created_at: OLD, last_activity_at: NOW, archived_at: null, waiting_question_id: null,
  waiting_options: [], waiting_since: null, open_request_id: null, seen_at: null, status: "offline", ...extra,
});
const message = (id: string): Message => ({
  id: `message-${id}`, thread_id: "handoff", sender: "master", recipient: "worker", sender_session: "task",
  recipient_session: "child", kind: "text", body: `Saved history for ${id}`, attachments: [], system_code: null,
  system_params: null, status: "interrupted", chain_id: "chain", hop: 0, in_reply_to: null, created_at: NOW,
  delivered_at: NOW, task_id: null, task_name: null, task_since: null, workflow_mark: null, workflow_request: false,
});
const thread: ThreadSummary = {
  id: "handoff", participant_a: "master", participant_b: "worker", session_a: "task", session_b: "child",
  last_message_at: NOW,
};
const emit = (event: UiEvent) => tauri.listeners.forEach((handler) => handler({ payload: event }));
const called = (command: string) => tauri.calls.filter(([name]) => name === command).map(([, args]) => args);

let root: Root;
let host: HTMLElement;
let sessions: SessionView[];
const column = (name: string) => host.querySelector(`[data-column="${name}"]`)!;
const drawer = () => host.querySelector(".drawer")!;
const titles = (scope: Element) => [...scope.querySelectorAll(".task-title")].map((el) => el.textContent);
function button(label: string, scope: Element = host): HTMLButtonElement {
  const found = [...scope.querySelectorAll("button")].find((b) =>
    b.textContent?.trim() === label || b.getAttribute("aria-label") === label ||
    b.querySelector(".trail-name")?.textContent === label);
  expect(found, `button ${label}`).toBeDefined();
  return found!;
}
const click = (element: HTMLElement) => act(async () => element.click());
const mount = () => act(async () => root.render(<App />));
function archive(archived_at: string | null) {
  sessions = sessions.map((s) => s.id === "task" || s.id === "child" ? { ...s, archived_at } : s);
  for (const s of sessions.filter((s) => s.id === "task" || s.id === "child")) emit({ type: "session_updated", session: s });
  return ["task", "child"];
}

describe("done tasks on the board", () => {
  beforeEach(() => {
    tauri.calls = [];
    tauri.listeners.clear();
    sessions = [
      session("task"),
      session("child", { bot_id: "worker", opened_by: "task" }),
      session("Previously done", { archived_at: OLD, last_activity_at: OLD }),
    ];
    tauri.answers = {
      get_settings: () => ({ setup_done: true, language_resolved: "en" }),
      list_bots: () => [bot("master"), bot("worker")],
      list_sessions: () => sessions,
      list_bot_threads: () => [thread],
      get_paused: () => false,
      usage_limits: () => [],
      detect_providers: () => [],
      list_skills: () => [],
      list_tasks: () => [],
      list_workflows: () => [],
      session_activity: () => [],
      session_glances: () => [],
      get_session_messages: (args) => [message((args as { sessionId: string }).sessionId)],
      get_thread_messages: () => [message("handoff")],
      archive_session: () => archive(NOW),
      restore_session: () => archive(null),
    };
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("marks done, opens read-only member and handoff history, then restores the task", async () => {
    await mount();
    expect(column("done")).toBeNull();
    expect(button("Show done").getAttribute("aria-expanded")).toBe("false");
    expect(titles(column("finished"))).toEqual(["task"]);
    await click(button("Mark done", column("finished")));
    expect(called("archive_session")).toEqual([{ sessionId: "task" }]);
    expect(titles(column("finished"))).toEqual([]);

    await click(button("Show done"));
    expect(button("Hide done").getAttribute("aria-expanded")).toBe("true");
    expect(titles(column("done"))).toEqual(["task", "Previously done"]);
    expect(column("done").querySelector(".board-count")?.textContent).toBe("2");
    expect(column("done").querySelector("details")).toBeNull();
    await click(button("Hide done"));
    expect(column("done")).toBeNull();
    await click(button("Show done"));
    await click(button("task", column("done")));
    expect(drawer().textContent).toContain("Saved history for task");
    expect(drawer().querySelector("textarea")).toBeNull();
    expect(drawer().textContent).not.toContain("Retry");
    expect(drawer().textContent).not.toContain("Mark done");
    await click(button("@worker", drawer()));
    expect(drawer().textContent).toContain("Saved history for child");
    await click(button("Hand-offs", drawer()));
    await click(drawer().querySelector<HTMLButtonElement>(".drawer-list button")!);
    expect(drawer().textContent).toContain("Saved history for handoff");
    expect(drawer().textContent).not.toContain("Retry");

    let finish!: () => void;
    tauri.answers.restore_session = () => new Promise((resolve) => {
      finish = () => resolve(archive(null));
    });
    const restore = button("Restore", drawer());
    await click(restore);
    expect(restore.disabled).toBe(true);
    await click(restore);
    expect(called("restore_session")).toEqual([{ sessionId: "task" }]);
    await act(async () => finish());
    expect(drawer()).toBeNull();
    expect(titles(column("done"))).toEqual(["Previously done"]);
    expect(titles(column("finished"))).toEqual(["task"]);
    expect(column("finished").textContent).toContain("@worker");
    expect(called("restart_session")).toEqual([]);
    expect(called("send_message")).toEqual([]);
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });

  it("keeps the done card and allows another attempt when restoring fails", async () => {
    sessions = [session("task", { archived_at: NOW })];
    tauri.answers.restore_session = () => Promise.reject(new Error("Restore failed"));
    await mount();
    await click(button("Show done"));
    await click(button("Restore", column("done")));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Restore failed");
    expect(titles(column("done"))).toEqual(["task"]);
    expect(button("Restore", column("done")).disabled).toBe(false);
    tauri.answers.restore_session = () => archive(null);
    await click(button("Restore", column("done")));
    expect(titles(column("done"))).toEqual([]);
    expect(titles(column("finished"))).toEqual(["task"]);
  });

  it("shows an empty Done column and resets its visibility on a later board visit", async () => {
    sessions = [session("task")];
    await mount();
    await click(button("Show done"));
    expect(column("done").textContent).toContain("Nothing here");
    expect(column("done").querySelector(".board-count")?.textContent).toBe("0");
    await click(button("Archive"));
    await click(button("Tasks"));
    expect(column("done")).toBeNull();
    expect(button("Show done").getAttribute("aria-expanded")).toBe("false");
  });
});
