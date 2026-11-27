import type { WorkItem } from "./api";
import { buildDoneTasks, buildTasks, type BoardInput, type BoardTask } from "./board";

export function taskProgress(parent: string, items: WorkItem[]) {
  const children = items.filter((w) => w.parent_id === parent);
  const finished = children.filter((w) => w.state === "finished" || w.state === "done");
  return { total: children.length, finished: finished.length,
    unsuccessful: finished.filter((w) => ["failed", "blocked", "cancelled"].includes(w.outcome ?? "")).length };
}

export function taskExecution(item: WorkItem, input: BoardInput): BoardTask | null {
  const members = input.sessions.filter((s) => item.session_ids.includes(s.id));
  const root = members.find((s) => s.id === item.primary_session_id);
  if (!root) return null;
  const sessions = members.map((s) => ({ ...s, title: s.id === root.id ? item.title : s.title,
    archived_at: item.archived_at, opened_by: s.id === root.id ? null : root.id }));
  const task = (item.archived_at ? buildDoneTasks({ sessions, activity: input.activity }) : buildTasks({ ...input, sessions }))[0];
  if (!task) return null;
  const column = item.archived_at ? "done" : item.state === "finished" ? "finished"
    : item.state === "needs_you" || task.column === "needs_you" ? "needs_you" : "running";
  return { ...task, workItem: item, column, kind: column === "finished" ? "finished" : column === "done" ? "done"
    : task.kind === "finished" ? "working" : task.kind, quietSince: item.archived_at ?? item.finished_at ?? task.quietSince };
}

export function pendingReason(item: WorkItem, items: WorkItem[]): "held" | "dependencies" | "queued" {
  if (item.manual_start) return "held";
  if (item.depends_on.some((id) => items.find((w) => w.id === id)?.execution?.state !== "succeeded")) return "dependencies";
  return "queued";
}
