// @vitest-environment happy-dom
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionView } from "../api";
import { buildTasks, type BoardInput } from "../board";
import i18n from "../i18n";
import { markDone } from "./Board";

const tauri = vi.hoisted(() => ({ asked: [] as string[], answer: true, invoked: [] as [string, unknown][] }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (message: string) => {
    tauri.asked.push(message);
    return Promise.resolve(tauri.answer);
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    tauri.invoked.push([command, args]);
    return Promise.resolve([]);
  },
}));

const s = (id: string, extra: Partial<SessionView> = {}): SessionView => ({
  id,
  bot_id: `bot-${id}`,
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

async function markDoneOf(sessions: SessionView[]) {
  const input: BoardInput = { sessions, activity: {}, glances: {}, attentionSince: {}, paused: false, limits: [], providers: {} };
  const [task] = buildTasks(input);
  await markDone(task, input.activity, input.glances, "Fix the login", i18n.t);
  return task;
}

describe("Mark done", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    tauri.asked = [];
    tauri.invoked = [];
    tauri.answer = true;
  });

  it("asks first on a Needs-you task with a member at work, and archives the root only on yes", async () => {
    const sessions = [
      s("m", { status: "working" }),
      s("f", { opened_by: "m", waiting_since: "2026-11-04T11:00:00.000Z", waiting_question_id: "q" }),
    ];
    tauri.answer = false;
    const task = await markDoneOf(sessions);
    expect(task.column).toBe("needs_you");
    expect(tauri.asked).toEqual(["Fix the login is still working. Marking it done stops it."]);
    expect(tauri.invoked).toEqual([]);

    tauri.answer = true;
    await markDoneOf(sessions);
    expect(tauri.invoked).toEqual([["archive_session", { sessionId: "m" }]]);
  });

  it("archives a quiet finished task without asking", async () => {
    const task = await markDoneOf([s("m"), s("f", { opened_by: "m" })]);
    expect(task.column).toBe("finished");
    expect(tauri.asked).toEqual([]);
    expect(tauri.invoked).toEqual([["archive_session", { sessionId: "m" }]]);
  });
});
