// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView, SessionActivity, SessionView } from "../api";
import { buildTasks } from "../board";
import i18n from "../i18n";
import TaskCard from "./TaskCard";

const tauri = vi.hoisted(() => ({
  sends: [] as { args: unknown; resolve: (v: unknown) => void; reject: (e: unknown) => void }[],
  others: [] as [string, unknown][],
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    if (command === "send_message") return new Promise((resolve, reject) => tauri.sends.push({ args, resolve, reject }));
    tauri.others.push([command, args]);
    return Promise.resolve(null);
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot: BotView = { project_id: "default",
  id: "b3",
  handle: "testing",
  display_name: "Testing",
  color: "#0d9488",
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status: "idle",
};
const session: SessionView = { project_id: "default",
  id: "t",
  bot_id: "b3",
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "2026-11-04T09:00:00.000Z",
  last_activity_at: "2026-11-04T09:00:00.000Z",
  archived_at: null,
  waiting_question_id: "q1",
  waiting_options: [],
  waiting_since: "2026-11-04T11:00:00.000Z",
  open_request_id: null,
  seen_at: null,
  status: "idle",
};
const activity: Record<string, SessionActivity> = {
  t: {
    session_id: "t",
    working_on: null,
    queued: 0,
    question: "Which one?",
    last_sent: null,
    cut_off: null,
    last_reply: null,
    last_for_user_at: null,
    run_of: null,
    workflow: null,
  },
};

let root: Root;
let host: HTMLElement;
const onError = vi.fn();

async function mount(holder: SessionView = session, seen: Record<string, SessionActivity> = activity) {
  const [task] = buildTasks({ sessions: [holder], activity: seen, glances: {}, attentionSince: {}, paused: false, limits: [], providers: {} });
  await act(async () =>
    root.render(
      <TaskCard
        task={task}
        bots={[bot]}
        sessions={[holder]}
        activity={seen}
        glances={{}}
        now={new Date("2026-11-04T12:00:00.000Z")}
        onOpen={() => {}}
        onMarkDone={() => {}}
        onError={onError}
      />,
    ),
  );
}

const box = () => host.querySelector("textarea")!;
async function type(text: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(box(), text);
    box().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function enter() {
  await act(async () => {
    box().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  });
}
const bodies = () => tauri.sends.map((s) => (s.args as { body: string }).body);

describe("the answer box", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    tauri.sends = [];
    tauri.others = [];
    onError.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("keeps what was typed while an answer is sending, and sends nothing more until it is sent", async () => {
    await mount();
    await type("The first one");
    await enter();
    expect(bodies()).toEqual(["The first one"]);
    expect(tauri.sends[0].args).toEqual({ botId: "b3", sessionId: "t", body: "The first one", files: [] });

    await type("The first one\nAnd one more thing");
    await enter();
    expect(bodies()).toEqual(["The first one"]);
    expect(box().value).toBe("The first one\nAnd one more thing");

    await act(async () => tauri.sends[0].resolve({}));
    expect(box().value).toBe("And one more thing");
    await enter();
    expect(bodies()).toEqual(["The first one", "And one more thing"]);
    expect(onError).not.toHaveBeenCalled();
  });

  it("empties once the answer is sent", async () => {
    await mount();
    await type("The first one");
    await enter();
    expect(box().value).toBe("The first one");
    await act(async () => tauri.sends[0].resolve({}));
    expect(box().value).toBe("");
  });

  it("keeps the answer when the send fails", async () => {
    await mount();
    await type("The first one");
    await enter();
    await act(async () => tauri.sends[0].reject(new Error("not delivered")));
    expect(box().value).toBe("The first one");
    expect(onError).toHaveBeenCalledWith(new Error("not delivered"));
    expect(tauri.others).toEqual([]);
  });

  it("keeps macOS's spelling and autocorrect off the box, and lets go of the keyboard on Enter", async () => {
    await mount();
    expect([box().getAttribute("spellcheck"), box().getAttribute("autocorrect")]).toEqual(["false", "off"]);
    expect(box().getAttribute("autocapitalize")).toBe("off");
    box().focus();
    expect(document.activeElement).toBe(box());
    await type("The first one");
    await enter();
    expect(document.activeElement).not.toBe(box());
  });

  it("marks the session that asked read once the answer is sent, as answering in its chat would", async () => {
    await mount();
    await type("The first one");
    await enter();
    expect(tauri.others).toEqual([]);
    await act(async () => tauri.sends[0].resolve({}));
    expect(tauri.others).toEqual([["mark_session_seen", { sessionId: "t" }]]);
    expect(onError).not.toHaveBeenCalled();
  });
});

describe("answer buttons", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    tauri.sends = [];
    tauri.others = [];
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });
  const withOptions = { ...session, waiting_options: ["Approve", "Change the plan"] };
  const options = () => Array.from(host.querySelectorAll<HTMLButtonElement>(".answer-options button"));
  const click = (b: HTMLElement) => act(async () => b.dispatchEvent(new MouseEvent("click", { bubbles: true })));

  it("shows a button per option", async () => {
    await mount(withOptions);
    expect(options().map((b) => b.textContent)).toEqual(["Approve", "Change the plan"]);
  });

  it("a press with an empty box sends the option to the holder", async () => {
    await mount(withOptions);
    await click(options()[0]);
    expect(tauri.sends.map((s) => s.args)).toEqual([{ botId: "b3", sessionId: "t", body: "Approve", files: [] }]);
  });

  it("a press with text sends the option and the text", async () => {
    await mount(withOptions);
    await type("use X");
    expect(options()[0].textContent).toBe("Approve with comment");
    await click(options()[0]);
    expect(bodies()).toEqual(["Approve\n\nuse X"]);
  });

  it("a holder without options shows no button", async () => {
    await mount();
    expect(options()).toEqual([]);
  });

  it("Try now asks the engine to try the holder's provider", async () => {
    await mount({ ...session, status: "limited", waiting_question_id: null, waiting_since: null });
    const button = [...host.querySelectorAll("button")].find((b) => b.textContent === "Try now")!;
    await click(button);
    expect(tauri.others).toEqual([["try_usage_limit_now", { provider: "claude" }]]);
  });
  it("a finished card's file opens it, and one that would run a program shows it in Finder", async () => {
    const finished = { ...session, waiting_question_id: null, waiting_since: null };
    const files = ["report.md", "setup.sh"];
    const reply = { message_id: "msg9", text: "Done.", files, at: "2026-11-04T11:00:00.000Z", raw: false };
    await mount(finished, { t: { ...activity.t, question: null, last_reply: reply } });
    const chips = [...host.querySelectorAll<HTMLButtonElement>(".task-file")];
    expect(chips.map((b) => [b.tagName, b.textContent])).toEqual([
      ["BUTTON", "report.md"],
      ["BUTTON", "setup.sh"],
    ]);
    await click(chips[0]);
    await click(chips[1]);
    expect(tauri.others).toEqual([
      ["open_attachment", { messageId: "msg9", index: 0, reveal: false }],
      ["open_attachment", { messageId: "msg9", index: 1, reveal: true }],
    ]);
  });
});
