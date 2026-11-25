import { describe, expect, it } from "vitest";
import type { Orchestration, OrchestrationExecution, SessionView } from "./api";
import type { BoardInput } from "./board";
import { buildOrchestrationBoard, orchestrationProgress } from "./orchestration";

const session = (id: string, extra: Partial<SessionView> = {}): SessionView => ({ project_id: "default", id, bot_id: id, number: 1,
  title: id, provider_session_id: null, session_config_version: null, opened_by: id === "master" ? null : "master",
  created_at: "2026-11-25", last_activity_at: "2026-11-25", archived_at: null, waiting_question_id: null,
  waiting_options: [], waiting_since: null, open_request_id: null, seen_at: null, status: "idle", ...extra });
const task = (key: string, extra: Partial<OrchestrationExecution> = {}): OrchestrationExecution => ({
  spec: { key, title: key, instructions: "Work", acceptance_criteria: ["Checked"], executor: { kind: "workflow", name: `Flow ${key}` }, depends_on: [], files: [] },
  state: "running", session_id: key, workflow_run_id: `run-${key}`, request_message_id: null, result_message_id: null, result: null,
  attachments: [], review: null, archived_at: null, session_ids: [key], ...extra,
});
const run = (tasks: OrchestrationExecution[]): Orchestration => ({ id: "process", project_id: "default", session_id: "master",
  request_message_id: "request", title: "Process", goal: "Goal", revision: 1, status: "active", tasks, executions: tasks,
  final_message_id: null, final_report: null, final_files: [], created_at: "2026-11-25", ended_at: null });
const input = (sessions: SessionView[]): BoardInput => ({ sessions, activity: {}, glances: {}, attentionSince: {}, paused: false, limits: [], providers: {} });

describe("orchestration board ownership", () => {
  it("shows a parent and separate workflow cards, including tasks without sessions", () => {
    const pending = task("c", { state: "waiting", session_id: null, session_ids: [], workflow_run_id: null });
    const grouped = buildOrchestrationBoard(input([session("master"), session("a"), session("b")]), [run([task("a"), task("b"), pending])]);
    expect(grouped.normal.map((t) => t.id)).toEqual(["master"]);
    expect(grouped.normal[0].members.map((s) => s.id)).toEqual(["master"]);
    expect(grouped.normal[0].column).toBe("running");
    expect(grouped.children.map((t) => t.task.spec.key)).toEqual(["a", "b", "c"]);
    expect(grouped.children[2].execution).toBeNull();
    expect(grouped.children[2].column).toBe("running");
  });

  it("keeps workflow steps within their own task and propagates attention to its parent", () => {
    const a = task("a", { session_ids: ["a", "step2"] });
    const grouped = buildOrchestrationBoard(input([session("master"), session("a"), session("step2", { waiting_question_id: "q", waiting_since: "now" }), session("b")]), [run([a, task("b")])]);
    expect(grouped.normal[0].column).toBe("needs_you");
    expect(grouped.children[0].execution?.members.map((s) => s.id)).toEqual(["a", "step2"]);
    expect(grouped.children[0].column).toBe("needs_you");
    expect(grouped.children[1].column).toBe("running");
  });

  it("does not finish the parent while Master is reviewing or preparing the final report", () => {
    const process = run([task("a", { state: "awaiting_review" })]);
    let board = buildOrchestrationBoard(input([session("master"), session("a")]), [process]);
    expect(board.children[0].column).toBe("running");
    expect(board.children[0].execution?.column).toBe("running");
    expect(board.normal[0].column).toBe("running");
    process.tasks[0].state = "succeeded";
    board = buildOrchestrationBoard(input([session("master"), session("a")]), [process]);
    expect(board.children[0].column).toBe("finished");
    expect(board.normal[0].column).toBe("running");
    process.status = "completed";
    expect(buildOrchestrationBoard(input([session("master"), session("a")]), [process]).normal[0].column).toBe("finished");
  });

  it("counts resolved tasks without treating failed or blocked work as successful", () => {
    const process = run([task("a", { state: "succeeded" }), task("b", { state: "failed" }), task("c", { state: "blocked", session_id: null, session_ids: [] })]);
    expect(orchestrationProgress(process)).toEqual({ total: 3, resolved: 3, succeeded: 1, failed: 1, blocked: 1, cancelled: 0 });
  });

  it("retains separate archived child cards and restores the same relationships", () => {
    const process = run([task("a", { state: "succeeded", archived_at: "then" }), task("b")]);
    const board = buildOrchestrationBoard(input([session("master"), session("a", { archived_at: "then" }), session("b")]), [process]);
    expect(board.normal).toHaveLength(1);
    expect(board.children.map((t) => t.column)).toEqual(["done", "running"]);
    process.tasks[0].archived_at = null;
    expect(buildOrchestrationBoard(input([session("master"), session("a"), session("b")]), [process]).children[0].column).toBe("finished");
  });
});
