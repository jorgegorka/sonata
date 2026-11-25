import type { Orchestration, OrchestrationExecution, OrchestrationTask } from "./api";
import { buildDoneTasks, buildTasks, type BoardInput, type BoardTask, type Column } from "./board";

export function isTaskTerminal(task: OrchestrationTask): boolean {
  return ["succeeded", "failed", "blocked", "cancelled"].includes(task.state);
}

export function orchestrationProgress(run: Orchestration) {
  return {
    total: run.tasks.length,
    resolved: run.tasks.filter(isTaskTerminal).length,
    succeeded: run.tasks.filter((t) => t.state === "succeeded").length,
    failed: run.tasks.filter((t) => t.state === "failed").length,
    blocked: run.tasks.filter((t) => t.state === "blocked").length,
    cancelled: run.tasks.filter((t) => t.state === "cancelled").length,
  };
}

export interface OrchestrationCard {
  id: string;
  run: Orchestration;
  task: OrchestrationExecution;
  execution: BoardTask | null;
  column: Column;
}

function executionTask(input: BoardInput, run: Orchestration, task: OrchestrationExecution): BoardTask | null {
  const sessions = input.sessions.filter((s) => task.session_ids.includes(s.id));
  const root = sessions.find((s) => s.id === task.session_id) ?? sessions[0];
  if (!root) return null;
  // Group the actual execution sessions; pending tasks do not fabricate sessions.
  const normalized = sessions.map((s) => ({ ...s, opened_by: s.id === root.id ? null : root.id }));
  const grouped = root.archived_at !== null ? buildDoneTasks({ sessions: normalized, activity: input.activity })
    : buildTasks({ ...input, sessions: normalized });
  const result = grouped.find((t) => t.id === root.id);
  if (!result) return null;
  const wrapped = { ...result, orchestration: { run, taskKey: task.spec.key } };
  if (!isTaskTerminal(task) && result.column === "finished") {
    return { ...wrapped, column: "running", kind: "orchestration", owes: null };
  }
  return wrapped;
}

export function buildOrchestrationBoard(input: BoardInput, runs: Orchestration[]): { normal: BoardTask[]; children: OrchestrationCard[] } {
  const latest = new Map(runs.map((r) => [r.session_id, r]));
  const owned = new Set(runs.flatMap((r) => r.executions.flatMap((t) => t.session_ids)));
  const parents = input.sessions.filter((s) => !owned.has(s.id)).map((s) => latest.has(s.id) ? { ...s, opened_by: null } : s);
  const children = runs.flatMap((run) => run.executions.map((task): OrchestrationCard => {
    const execution = executionTask(input, run, task);
    const parentArchived = input.sessions.find((s) => s.id === run.session_id)?.archived_at != null;
    const column: Column = parentArchived || task.archived_at !== null ? "done"
      : isTaskTerminal(task) ? "finished"
      : execution?.column === "needs_you" ? "needs_you" : "running";
    return { id: `${run.id}:${task.spec.key}`, run, task, execution, column };
  }));
  const normal = [...buildTasks({ ...input, sessions: parents }), ...buildDoneTasks({ sessions: parents, activity: input.activity })]
    .map((task) => {
      const run = latest.get(task.id);
      if (!run) return task;
      const result = { ...task, workflow: null, orchestration: { run } };
      if (task.column === "done") return result;
      if (task.kind === "question" || task.kind === "dialog" || task.kind === "cut_off") return result;
      if (run.status === "active") {
        const needs = children.some((c) => c.run.id === run.id && c.column === "needs_you");
        return { ...result, column: needs ? "needs_you" as const : "running" as const, kind: "orchestration" as const };
      }
      return { ...result, column: "finished" as const, kind: "finished" as const };
    });
  return { normal, children };
}
