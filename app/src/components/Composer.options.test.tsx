// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView } from "../api";
import { EMPTY_DRAFT, type Draft } from "../drafts";
import i18n from "../i18n";
import Composer from "./Composer";

const tauri = vi.hoisted(() => ({
  sends: [] as { args: unknown; resolve: (v: unknown) => void }[],
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    if (command === "send_message") return new Promise((resolve) => tauri.sends.push({ args, resolve }));
    return Promise.resolve(null);
  },
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = { id: "b1", handle: "plan", display_name: "Plan" } as BotView;

function Wrapper({ options, initial }: { options: string[]; initial: Draft }) {
  const [draft, setDraft] = useState(initial);
  return (
    <Composer
      bot={bot}
      sessionId="s1"
      onSent={() => {}}
      autoFocus={false}
      draft={draft}
      onDraftChange={(update) => setDraft(update)}
      onError={() => {}}
      options={options}
    />
  );
}

let root: Root;
let host: HTMLElement;

async function mount(options: string[], initial: Draft = EMPTY_DRAFT) {
  await act(async () => root.render(<Wrapper options={options} initial={initial} />));
}
const box = () => host.querySelector("textarea")!;
const buttons = () => Array.from(host.querySelectorAll<HTMLButtonElement>(".answer-options button"));
async function type(text: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(box(), text);
    box().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
const click = (b: HTMLElement) => act(async () => b.dispatchEvent(new MouseEvent("click", { bubbles: true })));
const sentArgs = () => tauri.sends.map((s) => s.args);

describe("answer buttons in the composer", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    tauri.sends = [];
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("shows no buttons without options", async () => {
    await mount([]);
    expect(host.querySelector(".answer-options")).toBeNull();
  });

  it("a button with an empty box sends the option", async () => {
    await mount(["Approve", "Change"]);
    expect(buttons().map((b) => b.textContent)).toEqual(["Approve", "Change"]);
    await click(buttons()[0]);
    expect(sentArgs()).toEqual([{ botId: "b1", sessionId: "s1", body: "Approve", files: [] }]);
  });

  it("a button with text sends the option and the text, and clears the box", async () => {
    await mount(["Approve"]);
    await type("use X");
    await click(buttons()[0]);
    expect(sentArgs()).toEqual([{ botId: "b1", sessionId: "s1", body: "Approve\n\nuse X", files: [] }]);
    expect(box().value).toBe("");
  });

  it("a button sends the box's files too", async () => {
    await mount(["Approve"], { text: "", files: ["/tmp/a.png"] });
    await click(buttons()[0]);
    expect(sentArgs()).toEqual([{ botId: "b1", sessionId: "s1", body: "Approve", files: ["/tmp/a.png"] }]);
  });

  it("the label says with comment while the box has text", async () => {
    await mount(["Approve"]);
    expect(buttons()[0].textContent).toBe("Approve");
    await type("hm");
    expect(buttons()[0].textContent).toBe("Approve with comment");
  });

  it("a second click while sending sends nothing", async () => {
    await mount(["Approve"]);
    await act(async () => {
      buttons()[0].dispatchEvent(new MouseEvent("click", { bubbles: true }));
      buttons()[0].dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(tauri.sends).toHaveLength(1);
  });
});
