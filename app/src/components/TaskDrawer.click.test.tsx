// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView, SessionView, ThreadSummary } from "../api";
import { buildTasks } from "../board";
import type { DrawerView } from "../drawer";
import i18n from "../i18n";
import TaskDrawer, { HandoffList } from "./TaskDrawer";

vi.mock("@tauri-apps/api/core", () => ({ invoke: () => Promise.resolve(null) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = (id: string, handle: string): BotView => ({ project_id: "default",
  id,
  handle,
  display_name: handle[0].toUpperCase() + handle.slice(1),
  color: "#4f46e5",
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status: "idle",
});
const bots = ["master", "feature", "testing", "review", "debug", "query"].map((h, i) => bot(`b${i + 1}`, h));

const s = (id: string, bot_id: string, opened_by: string | null, minute: number): SessionView => ({ project_id: "default",
  id,
  bot_id,
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by,
  created_at: `2026-11-04T09:0${minute}:00.000Z`,
  last_activity_at: "2026-11-04T10:00:00.000Z",
  archived_at: null,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "idle",
});

const sessions = bots.map((b, i) => s(`s${i + 1}`, b.id, i === 0 ? null : `s${i}`, i));
const [task] = buildTasks({ sessions, activity: {}, glances: {}, attentionSince: {}, paused: false, limits: [], providers: {} });

const handlers = {
  onShow: vi.fn(),
  onOpenFull: vi.fn(),
  onMarkDone: vi.fn(),
  onClose: vi.fn(),
};

let root: Root;
let host: HTMLElement;

async function mount(view: DrawerView, withTask = true) {
  await act(async () =>
    root.render(
      <TaskDrawer
        task={withTask ? task : null}
        drawer={view}
        bots={bots}
        sessions={sessions}
        activity={{}}
        now={new Date("2026-11-04T12:00:00.000Z")}
        onError={() => {}}
        {...handlers}
      >
        <p>the chat</p>
      </TaskDrawer>,
    ),
  );
}

const buttons = () => [...host.querySelectorAll("button")];
const button = (text: string) => {
  const found = buttons().find(
    (b) =>
      b.textContent === text ||
      b.getAttribute("aria-label") === text ||
      b.querySelector(".trail-name")?.textContent === text,
  );
  expect(found, text).toBeDefined();
  return found!;
};
const click = (el: Element) => act(async () => void el.dispatchEvent(new MouseEvent("click", { bubbles: true })));
const trailText = () => host.querySelector(".trail")!.textContent;

describe("the task drawer's controls", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    for (const handler of Object.values(handlers)) handler.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("the trail chooses the chat, and Hand-offs lists the threads", async () => {
    await mount({ root: "s1", show: { kind: "session", id: "s1" } });
    await click(button("@feature"));
    expect(handlers.onShow).toHaveBeenLastCalledWith({ kind: "session", id: "s2" });
    await click(button("Hand-offs"));
    expect(handlers.onShow).toHaveBeenLastCalledWith({ kind: "handoffs" });
  });

  it("the +n shows the whole trail", async () => {
    await mount({ root: "s1", show: { kind: "session", id: "s1" } });
    expect(trailText()).not.toContain("@debug");
    expect(trailText()).not.toContain("@query");
    await click(button("+2"));
    for (const handle of ["@master", "@feature", "@testing", "@review", "@debug", "@query"]) {
      expect(trailText()).toContain(handle);
    }
    expect(trailText()).not.toContain("+2");
    await click(button("@query"));
    expect(handlers.onShow).toHaveBeenLastCalledWith({ kind: "session", id: "s6" });
  });

  it("keeps the chat shown in the trail when it is past the first four", async () => {
    await mount({ root: "s1", show: { kind: "session", id: "s5" } });
    expect(trailText()).toContain("@debug");
    expect(trailText()).toContain("+2");
  });

  it("Open the full chat, Mark done and Close do what they say", async () => {
    await mount({ root: "s1", show: { kind: "session", id: "s2" } });
    await click(button("Open the full chat"));
    expect(handlers.onOpenFull).toHaveBeenCalledTimes(1);
    await click(button("Mark done"));
    expect(handlers.onMarkDone).toHaveBeenCalledTimes(1);
    await click(button("Close"));
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("a hand-off opens its thread", async () => {
    const thread: ThreadSummary = {
      id: "th",
      participant_a: "b1",
      participant_b: "b2",
      session_a: "s1",
      session_b: "s2",
      last_message_at: "2026-11-04T10:00:00.000Z",
    };
    const onOpen = vi.fn();
    await act(async () =>
      root.render(<HandoffList threads={[thread]} bots={bots} sessions={sessions} onOpen={onOpen} />),
    );
    await click(host.querySelector("button")!);
    expect(onOpen).toHaveBeenCalledWith("th");
  });
});
