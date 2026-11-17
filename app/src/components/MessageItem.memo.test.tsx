// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { Message } from "../api";
import i18n from "../i18n";
import MessageItem from "./MessageItem";

const drawn = vi.hoisted(() => ({ count: 0 }));
vi.mock("react-markdown", () => ({
  default: ({ children }: { children: string }) => {
    drawn.count++;
    return <p>{children}</p>;
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const message: Message = {
  id: "m1",
  thread_id: "t1",
  sender: "user",
  recipient: "b1",
  sender_session: null,
  recipient_session: null,
  kind: "text",
  body: "Hello",
  attachments: [],
  system_code: null,
  system_params: null,
  status: "delivered",
  chain_id: "c1",
  hop: 0,
  in_reply_to: null,
  created_at: "2026-11-04T10:00:00.000Z",
  delivered_at: null,
  task_id: null,
  task_name: null,
  task_since: null,
  workflow_mark: null,
  workflow_request: false,
};

describe("a message in a chat", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("is not drawn again when its chat draws again with the same message", async () => {
    const root = createRoot(document.createElement("div"));
    const onError = () => {};
    const draw = () => <MessageItem message={message} senderName="You" mine onError={onError} />;
    await act(async () => root.render(draw()));
    await act(async () => root.render(draw()));
    expect(drawn.count).toBe(1);
    await act(async () => root.unmount());
  });
});
