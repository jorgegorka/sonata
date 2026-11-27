import { beforeAll, describe, expect, it } from "vitest";
import type { BotView, PickedElement, SessionView } from "./api";
import i18n from "./i18n";
import {
  PICK_INTRO,
  PendingSessions,
  elementSummary,
  pickBody,
  pickStrings,
  pickTarget,
  splitPageElements,
} from "./pick";

const element: PickedElement = {
  url: "http://localhost:3000/orders",
  title: "Orders · Acme Admin",
  selector: "form#order-form > div.actions > button.btn-primary",
  tag: "button",
  text: "Save order",
  html: '<button type="submit" class="btn btn-primary" data-action="orders#save">Save order</button>',
};
const comment = 'Make this say "Save" and disable it until the form is valid.';

describe("the picked-element block", () => {
  it("writes the comment, the intro line and a page-element block", () => {
    expect(pickBody({ comment, element })).toBe(
      `${comment}\n\n${PICK_INTRO}\n\`\`\`page-element\nurl: http://localhost:3000/orders\n` +
        "title: Orders · Acme Admin\nselector: form#order-form > div.actions > button.btn-primary\n" +
        'tag: button\ntext: Save order\nhtml:\n<button type="submit" class="btn btn-primary" ' +
        'data-action="orders#save">Save order</button>\n```',
    );
  });
  it("reads its own block back", () => {
    expect(splitPageElements(pickBody({ comment, element }))).toEqual([
      { kind: "markdown", text: comment },
      { kind: "element", element },
    ]);
  });
  it("makes the fence longer than any backtick run in the page data", () => {
    const tricky = { ...element, html: "<pre>```\n```page-element\n````</pre>", text: "a ``` b" };
    const body = pickBody({ comment, element: tricky });
    expect(body).toContain("`````page-element\n");
    expect(splitPageElements(body)).toEqual([
      { kind: "markdown", text: comment },
      { kind: "element", element: tricky },
    ]);
  });
  it("keeps the one-line fields on one line", () => {
    const body = pickBody({ comment, element: { ...element, title: "A\n  B", text: "x\t\ny" } });
    expect(body).toContain("\ntitle: A B\n");
    expect(body).toContain("\ntext: x y\n");
  });
  it("finds blocks anywhere, with or without the intro line", () => {
    const block = pickBody({ comment: "c", element }).split("\n\n")[1];
    const bare = block.split("\n").slice(1).join("\n");
    const body = `Brief for @feature.\n\n${block}\n\nThen run the tests.\n\n${bare}`;
    expect(splitPageElements(body)).toEqual([
      { kind: "markdown", text: "Brief for @feature." },
      { kind: "element", element },
      { kind: "markdown", text: "Then run the tests." },
      { kind: "element", element },
    ]);
  });
  it("leaves a body without a well-formed block as markdown", () => {
    const good = pickBody({ comment, element });
    for (const body of [
      "Just text with ```code```",
      good.slice(0, -3),
      good.replace("\nurl: ", "\nuri: "),
      good.replace(/\nurl: [^\n]*/, ""),
    ]) {
      expect(splitPageElements(body)).toEqual([{ kind: "markdown", text: body }]);
    }
  });
});

describe("elementSummary", () => {
  it("sums an element up as tag, text and where it is", () => {
    expect(elementSummary(element)).toBe('⌖ button "Save order" · localhost:3000/orders');
    expect(elementSummary({ ...element, text: "", url: "https://a.test/" })).toBe("⌖ button · a.test");
    expect(elementSummary({ ...element, text: "x".repeat(80) })).toBe(
      `⌖ button "${"x".repeat(59)}…" · localhost:3000/orders`,
    );
  });
  it("cuts at code points and shows an unparsable url as is", () => {
    expect(elementSummary({ ...element, text: "😀".repeat(70), url: "not a url" })).toBe(
      `⌖ button "${"😀".repeat(59)}…" · not a url`,
    );
  });
});

describe("pickTarget", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  const feature: BotView = { project_id: "default",
    id: "b1",
    handle: "feature",
    display_name: "Feature",
    color: "#2563eb",
    provider: "claude",
    model: null,
    instructions: "",
    config_version: 1,
    skill_ids: [],
    created_at: "t",
    updated_at: "t",
    status: "idle",
  };
  const session = (id: string, extra: Partial<SessionView> = {}): SessionView => ({ project_id: "default",
    id,
    bot_id: "b1",
    number: 1,
    title: "Login fix",
    provider_session_id: null,
    session_config_version: null,
    opened_by: null,
    created_at: "t",
    last_activity_at: "2026-10-19T10:00:00.000Z",
    archived_at: null,
    waiting_question_id: null,
    waiting_options: [],
    waiting_since: null,
    open_request_id: null,
    seen_at: null,
    status: "idle",
    ...extra,
  });
  const live = session("s1");
  const archived = session("s2", { archived_at: "2026-10-19T11:00:00Z" });
  const orphan = session("s3", { bot_id: "gone" });
  const sessions = [live, archived, orphan];
  const target = (page: Parameters<typeof pickTarget>[0], archiving: string | null = null) =>
    pickTarget(page, sessions, [feature], archiving, i18n.t);

  it("is the session shown, named by the bot's handle and the session's label", () => {
    expect(target({ kind: "session", id: "s1" })).toEqual({
      botId: "b1",
      sessionId: "s1",
      label: "@feature · Login fix",
    });
  });
  it("does not create a conversation from a retired blank-chat route", () => {
    expect(target({ kind: "newSession", botId: "b1" })).toBeNull();
  });

  it("is none for an archived session, unless its own chat is archiving it", () => {
    expect(target({ kind: "session", id: "s2" })).toBeNull();
    expect(target({ kind: "session", id: "s2" }, "s2")).toEqual({
      botId: "b1",
      sessionId: "s2",
      label: "@feature · Login fix",
    });
  });
  it("is none where there is no chat with a composer", () => {
    expect(target({ kind: "settings" })).toBeNull();
    expect(target({ kind: "thread", id: "th1" })).toBeNull();
    expect(target({ kind: "edit", id: "b1" })).toBeNull();
    expect(target(null)).toBeNull();
    expect(target({ kind: "session", id: "unknown" })).toBeNull();
  });
  it("is none for a deleted bot", () => {
    expect(target({ kind: "session", id: "s3" })).toBeNull();
    expect(target({ kind: "newSession", botId: "gone" })).toBeNull();
  });
});

describe("pickStrings", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  it("translates every text of the comment box", () => {
    const strings = pickStrings(i18n.t);
    expect(Object.keys(strings).sort()).toEqual([
      "failed", "hint", "keys", "noTarget", "placeholder", "send", "sending", "tooLong",
    ]);
    for (const [field, text] of Object.entries(strings)) {
      expect(text.trim(), field).not.toBe("");
      expect(text, field).not.toBe(`pick.${field}`);
    }
  });
});

describe("PendingSessions", () => {
  it("holds a blank chat's send until it settles", async () => {
    const pending = new PendingSessions();
    let resolve: (id: string) => void = () => {};
    const p = new Promise<string>((r) => (resolve = r));
    expect(pending.track("b", p)).toBe(p);
    expect(pending.get("b")).toBe(p);
    expect(pending.get("other")).toBeUndefined();
    resolve("s1");
    await expect(p).resolves.toBe("s1");
    expect(pending.get("b")).toBeUndefined();
  });
  it("forgets a send that failed", async () => {
    const pending = new PendingSessions();
    const p = Promise.reject(new Error("no"));
    pending.track("b", p);
    expect(pending.get("b")).toBe(p);
    await expect(p).rejects.toThrow("no");
    expect(pending.get("b")).toBeUndefined();
  });
  it("keeps a newer send when an older one settles", async () => {
    const pending = new PendingSessions();
    const first = Promise.resolve("s1");
    let resolve: (id: string) => void = () => {};
    const second = new Promise<string>((r) => (resolve = r));
    pending.track("b", first);
    pending.track("b", second);
    await first;
    expect(pending.get("b")).toBe(second);
    resolve("s2");
    await second;
    expect(pending.get("b")).toBeUndefined();
  });
});
