import { renderToStaticMarkup } from "react-dom/server";
import { beforeAll, describe, expect, it } from "vitest";
import type { BotView, SessionActivity, SessionView } from "../api";
import i18n from "../i18n";
import Board from "./Board";
import type { BoardData } from "./useBoardData";

const bot = (id: string, handle: string, display_name: string): BotView => ({
  id,
  handle,
  display_name,
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
const bots = [bot("b1", "master", "Master"), bot("b2", "feature", "Feature")];

const NOW = new Date("2026-11-20T12:00:00.000Z");

const s = (id: string, bot_id: string, extra: Partial<SessionView> = {}): SessionView => ({
  id,
  bot_id,
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "2026-11-20T09:00:00.000Z",
  last_activity_at: "2026-11-20T11:00:00.000Z",
  archived_at: null,
  waiting_question_id: null,
  waiting_options: [],
  waiting_since: null,
  open_request_id: null,
  seen_at: null,
  status: "idle",
  ...extra,
});

const nothing = () => {};
const empty: BoardData = { activity: {}, glances: {}, attentionSince: {} };

function board(sessions: SessionView[], data: BoardData = empty, paused = false): string {
  return renderToStaticMarkup(
    <Board
      bots={bots}
      sessions={sessions}
      paused={paused}
      limits={[]}
      onTogglePause={nothing}
      onOpen={nothing}
      onNewTask={nothing}
      onError={nothing}
      data={data}
      now={NOW}
    />,
  );
}

function column(html: string, name: string): string {
  const match = new RegExp(`<section[^>]*data-column="${name}"[^>]*>([\\s\\S]*?)</section>`).exec(html);
  expect(match, `column ${name}`).not.toBeNull();
  return match![1];
}

const count = (html: string, part: string) => html.split(part).length - 1;
const countOf = (columnHtml: string) => /<span class="board-count">(\d+)<\/span>/.exec(columnHtml)?.[1];

describe("the task board", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("the board lists each task once in its column, with counts", () => {
    const html = board([s("m", "b1"), s("f", "b2", { opened_by: "m", status: "working" })]);
    expect(count(html, "<article")).toBe(1);
    const running = column(html, "running");
    expect(count(running, "<article")).toBe(1);
    expect(running).toContain(">Session 1</button>");
    expect(countOf(running)).toBe("1");
    expect(countOf(column(html, "needs_you"))).toBe("0");
    expect(countOf(column(html, "finished"))).toBe("0");
    expect(column(html, "finished")).toContain("Nothing here");
    expect(html.indexOf('data-column="running"')).toBeLessThan(html.indexOf('data-column="needs_you"'));
    expect(html.indexOf('data-column="needs_you"')).toBeLessThan(html.indexOf('data-column="finished"'));
  });

  it("an empty board offers a new task", () => {
    const html = board([]);
    expect(html).toContain("No tasks yet.");
    expect(count(html, ">New task</button>")).toBeGreaterThanOrEqual(1);
    expect(html).toContain("<h1>Tasks</h1>");
    expect(html).not.toContain("<article");
  });

  it("a read task quiet for over a week folds under Older, with Mark all done", () => {
    const old = { last_activity_at: "2026-11-01T10:00:00.000Z", seen_at: "2026-11-01T11:00:00.000Z" };
    const reply = (id: string): SessionActivity => ({
      session_id: id,
      working_on: null,
      queued: 0,
      question: null,
      last_sent: null,
      cut_off: null,
      last_reply: { message_id: `r-${id}`, text: `Reply of ${id}`, files: [], at: "2026-11-01T10:00:00.000Z", raw: false },
      last_for_user_at: "2026-11-01T10:00:00.000Z",
      run_of: null,
      workflow: null,
    });
    const html = board([s("a", "b1", old), s("b", "b2", old), s("c", "b1", { number: 2 })], {
      ...empty,
      activity: { a: reply("a"), b: reply("b") },
    });
    const finished = column(html, "finished");
    expect(count(finished, "<article")).toBe(3);
    expect(finished).toContain("<details");
    expect(finished).not.toContain("<details open");
    expect(finished).toContain("Older (2)");
    expect(finished).toContain(">Mark all done</button>");
    expect(finished.indexOf("Session 2")).toBeLessThan(finished.indexOf("Older (2)"));
    expect(countOf(finished)).toBe("1");
  });

  it("lays the drawer over the board, with the board's tasks and time", () => {
    const sessions = [s("m", "b1"), s("f", "b2", { opened_by: "m", status: "working" })];
    const html = renderToStaticMarkup(
      <Board
        bots={bots}
        sessions={sessions}
        paused={false}
        limits={[]}
        onTogglePause={nothing}
        onOpen={nothing}
        onNewTask={nothing}
        onError={nothing}
        data={empty}
        now={NOW}
        drawer={(tasks, now) => (
          <aside className="drawer">
            {`${tasks.map((t) => `${t.id}:${t.holder.id}`).join(",")} ${now.toISOString()}`}
          </aside>
        )}
      />,
    );
    expect(html).toContain(`<aside class="drawer">m:f ${NOW.toISOString()}</aside>`);
    expect(count(html, "<article")).toBe(1);
  });

  it("says the bots are paused, and offers to resume them", () => {
    expect(board([])).toContain(">Pause all bots</button>");
    const paused = board([], empty, true);
    expect(paused).toContain(">Resume bots</button>");
    expect(paused).toContain("Bots are paused.");
  });
});
