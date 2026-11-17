import { renderToStaticMarkup } from "react-dom/server";
import { beforeAll, describe, expect, it } from "vitest";
import type { BotView, Message, SessionView, ThreadSummary, WorkflowMark } from "../api";
import { EMPTY_DRAFT } from "../drafts";
import i18n from "../i18n";
import Chat from "./Chat";
import ThreadView from "./ThreadView";

const bot: BotView = {
  id: "b1",
  handle: "master",
  display_name: "Master",
  color: "#4f46e5",
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status: "idle",
};
const session: SessionView = {
  id: "m",
  bot_id: "b1",
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
};
const nothing = () => {};

function chat(onBack?: () => void, messages: Message[] = []): string {
  return renderToStaticMarkup(
    <Chat
      bot={bot}
      session={session}
      label="Session 1"
      cascade={0}
      onSent={nothing}
      onRemoved={nothing}
      onArchive={async () => {}}
      focusComposer={false}
      messages={messages}
      onError={nothing}
      onEdit={nothing}
      draft={EMPTY_DRAFT}
      onDraftChange={nothing}
      browserOpen={false}
      onToggleBrowser={nothing}
      onBack={onBack}
    />,
  );
}

describe("the way back to the board", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("the full chat shows ← Board, the drawer's chat does not", () => {
    expect(chat(nothing)).toContain("← Board");
    expect(chat()).not.toContain("← Board");
  });

  it("a bot thread on the full page shows ← Board too", () => {
    const thread: ThreadSummary = {
      id: "th",
      participant_a: "b1",
      participant_b: "b2",
      session_a: "m",
      session_b: null,
      last_message_at: "2026-11-04T10:00:00.000Z",
    };
    const view = (onBack?: () => void) =>
      renderToStaticMarkup(
        <ThreadView
          thread={thread}
          bots={[bot]}
          sessions={[session]}
          messages={[]}
          onError={nothing}
          browserOpen={false}
          onToggleBrowser={nothing}
          onBack={onBack}
        />,
      );
    expect(view(nothing)).toContain("← Board");
    expect(view()).not.toContain("← Board");
  });
});

describe("who sent a message", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  const message = (id: string, sender: string, body: string, workflow_mark: WorkflowMark | null = null): Message => ({
    id,
    thread_id: "t1",
    sender,
    recipient: sender === "user" ? "b1" : "user",
    sender_session: sender === "user" ? null : "m",
    recipient_session: sender === "user" ? "m" : null,
    kind: "text",
    body,
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
    workflow_mark,
    workflow_request: false,
  });
  const mark = (role: WorkflowMark["role"]): WorkflowMark => ({
    run_id: "r1",
    visit_id: "v1",
    workflow: "Ticket to merge request",
    step: "Review",
    visit: 1,
    role,
    outcome: null,
  });
  const senders = (html: string) =>
    [...html.matchAll(/<div class="message-meta"><strong>([^<]*)<\/strong>/g)].map((m) => m[1]);

  it("the user's own words are You's; what a workflow run sent in the user's name is Sonata's", () => {
    const html = chat(undefined, [
      message("a", "user", "Build it."),
      message("b", "user", "Step request.", mark("request")),
      message("c", "user", "Step reminder.", mark("reminder")),
      message("d", "user", "Run finished.", mark("finished")),
      message("e", "b1", "Done.", mark("result")),
    ]);
    expect(senders(html)).toEqual(["You", "Sonata", "Sonata", "Sonata", "Master"]);
    expect(html).toContain("Sonata reminded the bot of step Review");
  });
});
