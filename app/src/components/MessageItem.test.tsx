import { renderToStaticMarkup } from "react-dom/server";
import { beforeAll, describe, expect, it } from "vitest";
import type { Message, PickedElement, WorkflowMark } from "../api";
import i18n from "../i18n";
import { pickBody } from "../pick";
import MessageItem from "./MessageItem";

const element: PickedElement = {
  url: "http://localhost:3000/orders",
  title: "Orders · Acme Admin",
  selector: "form#order-form > button.btn-primary",
  tag: "button",
  text: "Save order",
  html: "<button>Save order</button>",
};

const message = (body: string): Message => ({
  id: "m1",
  thread_id: "t1",
  sender: "user",
  recipient: "b1",
  sender_session: null,
  recipient_session: null,
  kind: "text",
  body,
  attachments: [],
  system_code: null,
  system_params: null,
  status: "delivered",
  chain_id: "c1",
  hop: 0,
  in_reply_to: null,
  created_at: "2026-10-19T10:00:00.000Z",
  delivered_at: null,
  task_id: null,
  task_name: null,
  task_since: null,
  workflow_mark: null,
  workflow_request: false,
});

const render = (body: string) =>
  renderToStaticMarkup(<MessageItem message={message(body)} senderName="You" mine onError={() => {}} />);

describe("a picked element in a message", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("shows the comment and a compact expandable line, not the block", () => {
    const html = render(pickBody({ comment: "Make this say Save", element }));
    expect(html).toContain("Make this say");
    expect(html).toContain('<details class="picked-element">');
    expect(html).toContain("⌖ button &quot;Save order&quot; · localhost:3000/orders");
    expect(html).toContain("Selector");
    expect(html).not.toContain("page-element");
    expect(html).not.toContain("quoted page data");
  });

  it("renders a plain body as before", () => {
    const html = render("Some **bold** text");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).not.toContain("picked-element");
  });
});

describe("workflow marks label the message", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  const mark = (extra: Partial<WorkflowMark>): WorkflowMark => ({
    run_id: "r1",
    visit_id: "v1",
    workflow: "Ticket to merge request",
    step: "Review",
    visit: 2,
    role: "request",
    outcome: null,
    ...extra,
  });
  const marked = (workflow_mark: WorkflowMark, mine = true) =>
    renderToStaticMarkup(
      <MessageItem
        message={{ ...message("The body"), workflow_mark }}
        senderName={mine ? "You" : "Review"}
        mine={mine}
        onError={() => {}}
      />,
    );
  const label = (html: string) => /<div class="muted task-run-label">([^<]*)<\/div>/.exec(html)?.[1] ?? null;

  it("request", () => {
    const html = marked(mark({ role: "request" }));
    expect(label(html)).toBe("Ticket to merge request · step Review · visit 2");
    expect(html).toContain("The body");
  });

  it("reminder", () => {
    expect(label(marked(mark({ role: "reminder" })))).toBe("Sonata reminded the bot of step Review");
  });

  it("result", () => {
    const chosen = marked(mark({ role: "result", outcome: "approved" }), false);
    expect(label(chosen)).toBe("Result of step Review → approved");
    expect(label(marked(mark({ role: "result" }), false))).toBe("Result of step Review");
  });

  it("finished", () => {
    expect(label(marked(mark({ role: "finished" })))).toBe("Ticket to merge request finished");
  });

  it("no mark, no label", () => {
    expect(label(render("Plain"))).toBeNull();
  });

  it("a workflow request from the Workflows page", () => {
    const html = renderToStaticMarkup(
      <MessageItem
        message={{ ...message("An SEO workflow"), workflow_request: true }}
        senderName="You"
        mine
        onError={() => {}}
      />,
    );
    expect(label(html)).toBe("Sent from the Workflows page to make a new workflow");
    expect(html).toContain("An SEO workflow");
  });
});
