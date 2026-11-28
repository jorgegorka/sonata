// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { BotView, SessionView } from "../api";
import i18n from "../i18n";
import Rail from "./Rail";

const bot = (id: string, display_name: string, status: BotView["status"]): BotView => ({ project_id: "default",
  id,
  handle: display_name.toLowerCase(),
  display_name,
  color: "#2563eb",
  provider: "claude",
  model: null,
  instructions: "",
  config_version: 1,
  skill_ids: [],
  created_at: "t",
  updated_at: "t",
  status,
});
const planner = bot("b1", "Planner", "working");
const writer = bot("b2", "Writer", "idle");

const session = (id: string, botId: string, waiting_since: string | null = null): SessionView => ({ project_id: "default",
  id,
  bot_id: botId,
  number: 1,
  title: null,
  provider_session_id: null,
  session_config_version: null,
  opened_by: null,
  created_at: "t",
  last_activity_at: "2026-10-16T10:00:00.000Z",
  archived_at: null,
  waiting_question_id: waiting_since ? `q-${id}` : null,
  waiting_options: [],
  waiting_since,
  open_request_id: null,
  seen_at: null,
  status: "idle",
});

const nothing = () => {};

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function rail(sessions: SessionView[], open: { board?: boolean; workflows?: boolean } = {}): string {
  return renderToStaticMarkup(
    <Rail
      bots={[planner, writer]}
      sessions={sessions}
      boardOpen={open.board ?? false}
      workflowsOpen={open.workflows ?? false}
      tasksOpen={false}
      archiveOpen={false}
      settingsOpen={false}
      onBoard={nothing}
      onBot={nothing}
      onNewBot={nothing}
      onWorkflows={nothing}
      onTasks={nothing}
      onArchive={nothing}
      onSettings={nothing}
    />,
  );
}

describe("the rail", () => {
  beforeAll(() => i18n.changeLanguage("en"));

  it("the rail shows each bot with its status and its waiting ring", () => {
    const html = rail([session("s1", "b1", "2026-10-16T10:00:00Z"), session("s2", "b2")]);
    expect(html).toContain('aria-label="Planner · Working… · Waiting for your answer"');
    expect(html).toContain('aria-label="Writer · Idle"');
    expect(html).toContain('class="status-dot working"');
    expect(html).toContain('class="status-dot idle"');
    expect(html.match(/class="waiting-mark"/g)).toHaveLength(1);
  });

  it("the rail's entry for scheduled prompts reads Recurring tasks", () => {
    const html = rail([]);
    expect(html).toContain('aria-label="Recurring tasks"');
    expect(html).toContain('title="Recurring tasks"');
  });

  it("lists the board, the bots, the new bot, then Workflows, Recurring tasks, Archive and Settings", () => {
    const html = rail([]);
    const names = [
      "Tasks",
      "Planner · Working…",
      "Writer · Idle",
      "New bot",
      "Workflows",
      "Recurring tasks",
      "Archive",
      "Settings",
    ];
    const order = names.map((name) => html.indexOf(`aria-label="${name}"`));
    expect(order.every((at) => at >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });

  it("shows the board's entry selected while the board is open", () => {
    expect(rail([], { board: true })).toMatch(/class="[^"]*\bselected\b[^"]*"[^>]*aria-label="Tasks"/);
    expect(rail([])).not.toContain("selected");
  });

  it("the new entry calls onWorkflows and shows as open", async () => {
    expect(rail([], { workflows: true })).toMatch(/class="[^"]*\bselected\b[^"]*"[^>]*aria-label="Workflows"[^>]*>/);
    expect(rail([])).not.toContain("selected");

    const onWorkflows = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    await act(async () =>
      root.render(
        <Rail
          bots={[]}
          sessions={[]}
          boardOpen={false}
          workflowsOpen={false}
          tasksOpen={false}
          archiveOpen={false}
          settingsOpen={false}
          onBoard={nothing}
          onBot={nothing}
          onNewBot={nothing}
          onWorkflows={onWorkflows}
          onTasks={nothing}
          onArchive={nothing}
          onSettings={nothing}
        />,
      ),
    );
    const entry = host.querySelector<HTMLButtonElement>('button[aria-label="Workflows"]')!;
    await act(async () => void entry.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(onWorkflows).toHaveBeenCalledTimes(1);
    await act(async () => root.unmount());
    host.remove();
  });
});
