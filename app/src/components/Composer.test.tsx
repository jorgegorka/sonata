// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { BotView, SessionView } from "../api";
import { EMPTY_DRAFT } from "../drafts";
import i18n from "../i18n";
import Composer from "./Composer";

const tauri = vi.hoisted(() => ({ invoked: [] as string[] }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string) => {
    tauri.invoked.push(command);
    return Promise.resolve(null);
  },
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bot = { id: "b1", handle: "feature", display_name: "Feature" } as BotView;
const started = { id: "s9", bot_id: "b1" } as SessionView;

describe("the composer's send", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("a send override is used and its session passed on", async () => {
    const send = vi.fn((_body: string, _files: string[]) => Promise.resolve(started));
    const onSent = vi.fn();
    function Wrapper() {
      const [draft, setDraft] = useState({ ...EMPTY_DRAFT, files: ["/tmp/ticket.md"] });
      return (
        <Composer
          bot={bot}
          sessionId={null}
          onSent={onSent}
          autoFocus={false}
          draft={draft}
          onDraftChange={setDraft}
          onError={() => {}}
          options={[]}
          send={send}
          placeholder="Write the request for Ticket to merge request…"
        />
      );
    }
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    await act(async () => root.render(<Wrapper />));
    const box = host.querySelector("textarea")!;
    expect(box.getAttribute("placeholder")).toBe("Write the request for Ticket to merge request…");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(box, "TASK-1234");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    expect(send).toHaveBeenCalledWith("TASK-1234", ["/tmp/ticket.md"]);
    expect(tauri.invoked).not.toContain("send_message");
    expect(onSent).toHaveBeenCalledWith(started);
    expect(box.value).toBe("");
    await act(async () => root.unmount());
    host.remove();
  });
});
