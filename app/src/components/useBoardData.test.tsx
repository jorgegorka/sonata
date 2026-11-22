// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Message, SessionActivity, SessionView, UiEvent } from "../api";
import { useBoardData, type BoardData } from "./useBoardData";

const tauri = vi.hoisted(() => ({
  calls: [] as [string, unknown][],
  hear: null as ((e: { payload: unknown }) => void) | null,
  answers: {} as Record<string, (args: unknown) => Promise<unknown>>,
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    tauri.calls.push([command, args]);
    return tauri.answers[command]?.(args) ?? Promise.resolve([]);
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_event: string, handler: (e: { payload: unknown }) => void) => {
    tauri.hear = handler;
    return Promise.resolve(() => {});
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const s = (id: string, extra: Partial<SessionView> = {}): SessionView => ({
  id,
  bot_id: "b1",
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "2026-11-04T09:00:00.000Z",
  last_activity_at: "2026-11-04T09:00:00.000Z",
  archived_at: null,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "idle",
  ...extra,
});
const activity = (id: string, queued = 0): SessionActivity => ({
  session_id: id,
  working_on: null,
  queued,
  question: null,
  last_sent: null,
  cut_off: null,
  last_reply: null,
  last_for_user_at: null,
  run_of: null,
  workflow: null,
});

const called = (command: string) => tauri.calls.filter(([c]) => c === command).map(([, args]) => args);
const hear = (event: UiEvent) => tauri.hear!({ payload: event });
const status = (id: string): UiEvent => ({ type: "session_status", session_id: id, bot_id: "b1", status: "working" });

let root: Root;
let data: BoardData;
let visibility: DocumentVisibilityState;

function Probe({ sessions, onError, includeArchived }: { sessions: SessionView[]; onError: (e: unknown) => void; includeArchived: boolean }) {
  data = useBoardData(sessions, onError, includeArchived);
  return null;
}
async function mount(sessions: SessionView[], onError: (e: unknown) => void = () => {}, includeArchived = false) {
  await act(async () => root.render(<Probe sessions={sessions} onError={onError} includeArchived={includeArchived} />));
}
async function wait(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

describe("the board's data", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    tauri.calls = [];
    tauri.answers = {};
    tauri.hear = null;
    visibility = "visible";
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
    root = createRoot(document.createElement("div"));
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("reads every session on the board on mount, then what the events name, in one call", async () => {
    tauri.answers.session_activity = (args) =>
      Promise.resolve((args as { sessionIds: string[] }).sessionIds.map((id) => activity(id, 1)));
    await mount([s("a"), s("b", { archived_at: "x" })]);
    expect(called("session_activity")).toEqual([{ sessionIds: ["a"] }]);
    expect(Object.keys(data.activity)).toEqual(["a"]);

    hear(status("a"));
    const message = { sender_session: "a", recipient_session: "c" } as Message;
    hear({ type: "message_added", message });
    await wait(299);
    expect(called("session_activity")).toHaveLength(1);
    await wait(1);
    expect(called("session_activity")).toEqual([{ sessionIds: ["a"] }, { sessionIds: ["a", "c"] }]);
    expect(Object.keys(data.activity).sort()).toEqual(["a", "c"]);
  });

  it("loads done summaries when revealed, reuses them, and never polls their terminals", async () => {
    tauri.answers.session_activity = (args) =>
      Promise.resolve((args as { sessionIds: string[] }).sessionIds.map((id) => activity(id)));
    const sessions = [s("active"), s("done", { archived_at: "x", status: "working" })];
    await mount(sessions);
    expect(called("session_activity")).toEqual([{ sessionIds: ["active"] }]);
    await mount(sessions, undefined, true);
    expect(called("session_activity")).toEqual([{ sessionIds: ["active"] }, { sessionIds: ["done"] }]);
    expect(data.activity.done).toBeDefined();
    await mount(sessions);
    await mount(sessions, undefined, true);
    expect(called("session_activity")).toHaveLength(2);
    await wait(3000);
    expect(called("session_glances").every((args) => !(args as { sessionIds: string[] }).sessionIds.includes("done"))).toBe(true);
  });

  it("keeps the newest answer when an older one comes back last", async () => {
    const answers: ((list: SessionActivity[]) => void)[] = [];
    tauri.answers.session_activity = () => new Promise((resolve) => answers.push(resolve));
    await mount([s("a")]);
    hear(status("a"));
    await wait(300);
    expect(answers).toHaveLength(2);
    await act(async () => answers[1]([activity("a", 2)]));
    await act(async () => answers[0]([activity("a", 1)]));
    expect(data.activity.a.queued).toBe(2);
  });

  it("drops a session the answer leaves out", async () => {
    tauri.answers.session_activity = () => Promise.resolve([activity("a")]);
    await mount([s("a")]);
    expect(data.activity.a).toBeDefined();
    tauri.answers.session_activity = () => Promise.resolve([]);
    hear(status("a"));
    await wait(300);
    expect(data.activity.a).toBeUndefined();
  });

  it("glances at the screens with a running CLI, but not while the window is hidden", async () => {
    tauri.answers.session_glances = () =>
      Promise.resolve([{ session_id: "w", step: "Reading files", background_work: false, tail: null }]);
    await mount([s("w", { status: "working" }), s("o", { status: "offline" })]);
    expect(called("session_glances")).toEqual([{ sessionIds: ["w"] }]);
    expect(data.glances.w.step).toBe("Reading files");

    visibility = "hidden";
    await wait(3000);
    expect(called("session_glances")).toHaveLength(1);
    visibility = "visible";
    await wait(3000);
    expect(called("session_glances")).toHaveLength(2);
  });

  it("keeps the glances it has when a new glance sees the same", async () => {
    const step = (text: string) => [{ session_id: "w", step: text, background_work: false, tail: null }];
    tauri.answers.session_glances = () => Promise.resolve(step("Reading files"));
    await mount([s("w", { status: "working" })]);
    const first = data.glances;
    await wait(3000);
    expect(called("session_glances")).toHaveLength(2);
    expect(data.glances).toBe(first);
    tauri.answers.session_glances = () => Promise.resolve(step("Running the specs"));
    await wait(3000);
    expect(data.glances).not.toBe(first);
    expect(data.glances.w.step).toBe("Running the specs");
  });

  it("a glance that fails only warns; an activity read that fails is reported", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const onError = vi.fn();
    tauri.answers.session_glances = () => Promise.reject(new Error("screen"));
    tauri.answers.session_activity = () => Promise.reject(new Error("store"));
    await mount([s("w", { status: "working" })], onError);
    expect(warn).toHaveBeenCalled();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(onError.mock.calls[0][0]).toEqual(new Error("store"));
  });

  it("notes when a dialog was first seen", async () => {
    await mount([s("d", { status: "needs_attention" }), s("i")]);
    expect(Object.keys(data.attentionSince)).toEqual(["d"]);
  });
});
