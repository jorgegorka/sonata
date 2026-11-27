import type { Glance, Orchestration, Provider, RunGlance, SessionActivity, SessionView, UiEvent, WorkItem, UsageLimit } from "./api";
import { isArchived, isWaiting } from "./state";

export type Column = "needs_you" | "running" | "finished" | "done";
export type CardKind =
  | "orchestration"
  | "question"
  | "dialog"
  | "cut_off"
  | "workflow"
  | "starting"
  | "working"
  | "queued"
  | "paused"
  | "limited"
  | "background"
  | "run_waits"
  | "finished"
  | "done";

export interface BoardInput {
  sessions: SessionView[];
  activity: Record<string, SessionActivity>;
  glances: Record<string, Glance>;
  attentionSince: Record<string, string>;
  paused: boolean;
  limits: UsageLimit[];
  providers: Record<string, Provider>;
}

export interface BoardTask {
  workItem?: WorkItem;
  orchestration?: { run: Orchestration; taskKey?: string };
  id: string;
  root: SessionView;
  members: SessionView[];
  holder: SessionView;
  column: Column;
  kind: CardKind;
  since: string | null;
  quietSince: string;
  unread: boolean;
  needing: { id: string; kind: NeedsKind }[];
  owes: SessionView | null;
  recurring: string | null;
  limit: UsageLimit | null;
  workflow: RunGlance | null;
}

export type NeedsKind = "question" | "dialog" | "cut_off";
type RunningKind = "starting" | "working" | "queued" | "paused" | "limited" | "background";

export function taskRoots(sessions: SessionView[]): SessionView[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  return sessions.filter((s) => {
    if (isArchived(s)) return false;
    const opener = s.opened_by === null ? undefined : byId.get(s.opened_by);
    return opener === undefined || isArchived(opener);
  });
}

export function membersOf(root: SessionView, sessions: SessionView[]): SessionView[] {
  const seen = new Set([root.id]);
  const found: SessionView[] = [root];
  for (let i = 0; i < found.length; i++) {
    for (const s of sessions) {
      if (s.opened_by === found[i].id && !isArchived(s) && !seen.has(s.id)) {
        seen.add(s.id);
        found.push(s);
      }
    }
  }
  const order = new Map(sessions.map((s, i) => [s.id, i]));
  const rest = found
    .slice(1)
    .sort((a, b) => a.created_at.localeCompare(b.created_at) || order.get(a.id)! - order.get(b.id)!);
  return [root, ...rest];
}

export function isUnread(members: SessionView[], activity: Record<string, SessionActivity>): boolean {
  return members.some((m) => {
    const at = activity[m.id]?.last_for_user_at ?? null;
    return at !== null && (m.seen_at === null || at > m.seen_at);
  });
}

export function hasWorkGoingOn(
  task: BoardTask,
  activity: Record<string, SessionActivity>,
  glances: Record<string, Glance>,
): boolean {
  return task.members.some((m) => {
    const a = activity[m.id];
    return (
      m.status === "starting" ||
      m.status === "working" ||
      m.status === "needs_attention" ||
      (a !== undefined && (a.working_on !== null || a.queued > 0)) ||
      glances[m.id]?.background_work === true
    );
  });
}

function needsKind(m: SessionView, activity: Record<string, SessionActivity>): NeedsKind | null {
  if (isWaiting(m)) return "question";
  if (m.status === "needs_attention") return "dialog";
  if (activity[m.id]?.cut_off) return "cut_off";
  return null;
}

function limitOf(m: SessionView, input: BoardInput): UsageLimit | null {
  const provider = input.providers[m.bot_id];
  return input.limits.find((l) => l.provider === provider) ?? null;
}

function runOf(members: SessionView[], activity: Record<string, SessionActivity>): RunGlance | null {
  const runs = members
    .map((m) => activity[m.id]?.workflow ?? null)
    .filter((g): g is RunGlance => g !== null)
    .reverse();
  return runs.find((g) => g.status === "paused") ?? runs.find((g) => g.status === "running") ?? runs[0] ?? null;
}

function runningKind(m: SessionView, input: BoardInput): RunningKind | null {
  if (m.status === "starting") return "starting";
  if (m.status === "working") return "working";
  if (m.status === "limited") return "limited";
  if ((input.activity[m.id]?.queued ?? 0) > 0) {
    if (limitOf(m, input) !== null) return "limited";
    return input.paused ? "paused" : "queued";
  }
  if (m.status === "idle" && input.glances[m.id]?.background_work) return "background";
  return null;
}

function sinceOf(m: SessionView, kind: CardKind, input: BoardInput, quietSince: string): string | null {
  switch (kind) {
    case "question":
      return m.waiting_since;
    case "dialog":
      return input.attentionSince[m.id] ?? null;
    case "cut_off":
      return input.activity[m.id]?.cut_off?.at ?? null;
    case "working":
      return input.activity[m.id]?.working_on?.at ?? null;
    case "finished":
      return quietSince;
    default:
      return null;
  }
}

export function buildTasks(input: BoardInput): BoardTask[] {
  return taskRoots(input.sessions).map((root) => {
    const members = membersOf(root, input.sessions);
    const needs = members.flatMap((m) => {
      const kind = needsKind(m, input.activity);
      return kind === null ? [] : [{ m, kind, since: sinceOf(m, kind, input, "") }];
    });
    const running = members.flatMap((m) => {
      const kind = runningKind(m, input);
      return kind === null ? [] : [{ m, kind }];
    });
    const quietSince = members.reduce((newest, m) => (m.last_activity_at > newest ? m.last_activity_at : newest), "");
    const workflow = runOf(members, input.activity);

    let column: Column;
    let holder: SessionView;
    let kind: CardKind;
    if (needs.length > 0) {
      const first = needs.reduce((best, n) =>
        n.since !== null && (best.since === null || n.since < best.since) ? n : best,
      );
      column = "needs_you";
      ({ m: holder, kind } = first);
    } else if (workflow?.status === "paused") {
      column = "needs_you";
      kind = "workflow";
      holder = members.find((m) => m.id === workflow.holder) ?? root;
    } else if (running.length > 0) {
      column = "running";
      ({ m: holder, kind } = running[running.length - 1]);
    } else if (workflow?.status === "running") {
      column = "running";
      holder = root;
      kind = "run_waits";
    } else {
      column = "finished";
      holder = root;
      kind = "finished";
    }
    return {
      id: root.id,
      root,
      members,
      holder,
      column,
      kind,
      since: sinceOf(holder, kind, input, quietSince),
      quietSince,
      unread: isUnread(members, input.activity),
      needing: needs.map(({ m, kind }) => ({ id: m.id, kind })),
      owes: column === "finished" ? (members.find((m) => m.open_request_id !== null) ?? null) : null,
      recurring: input.activity[root.id]?.run_of ?? null,
      limit: kind === "limited" ? limitOf(holder, input) : null,
      workflow,
    };
  });
}

export function buildDoneTasks({ sessions, activity }: Pick<BoardInput, "sessions" | "activity">): BoardTask[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const groups = new Map<string, { root: SessionView; members: SessionView[] }>();
  for (const session of sessions.filter(isArchived)) {
    let root = session;
    let parent = session.opened_by;
    const seen = new Set([session.id]);
    // Restore follows the whole tree, but only restores sessions with the same archive mark.
    while (parent !== null && !seen.has(parent)) {
      seen.add(parent);
      const ancestor = byId.get(parent);
      if (!ancestor) break;
      if (ancestor.archived_at === session.archived_at) root = ancestor;
      parent = ancestor.opened_by;
    }
    const group = groups.get(root.id) ?? { root, members: [] };
    group.members.push(session);
    groups.set(root.id, group);
  }
  return [...groups.values()].map(({ root, members }) => ({
    id: root.id,
    root,
    members: [root, ...members.filter((m) => m.id !== root.id).sort((a, b) =>
      a.created_at.localeCompare(b.created_at) || a.id.localeCompare(b.id),
    )],
    holder: root,
    column: "done",
    kind: "done",
    since: root.archived_at!,
    quietSince: root.archived_at!,
    unread: isUnread(members, activity),
    needing: [],
    owes: null,
    recurring: activity[root.id]?.run_of ?? null,
    limit: null,
    workflow: runOf(members, activity),
  }));
}

export const FOLD_DAYS = 7,
  TRAIL_MAX = 4,
  GLANCE_MS = 3000,
  ACTIVITY_BATCH_MS = 300;

const byId = (a: BoardTask, b: BoardTask) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

function ordered(tasks: BoardTask[], column: Column): BoardTask[] {
  const cmp = (a: BoardTask, b: BoardTask): number => {
    if (column === "finished" || column === "done") {
      if (column === "finished" && a.unread !== b.unread) return a.unread ? -1 : 1;
      if (a.quietSince !== b.quietSince) return a.quietSince > b.quietSince ? -1 : 1;
    } else if (a.since !== b.since) {
      if (a.since === null) return 1;
      if (b.since === null) return -1;
      return a.since < b.since ? -1 : 1;
    }
    return byId(a, b);
  };
  return [...tasks].sort(cmp);
}

export function columnTasks(tasks: BoardTask[], column: Column, now: Date): { shown: BoardTask[]; older: BoardTask[] } {
  const inColumn = ordered(
    tasks.filter((t) => t.column === column),
    column,
  );
  if (column !== "finished") return { shown: inColumn, older: [] };
  const cutoff = now.getTime() - FOLD_DAYS * 86_400_000;
  const folded = (t: BoardTask) => !t.unread && new Date(t.quietSince).getTime() < cutoff;
  return {
    shown: inColumn.filter((t) => !folded(t)),
    older: inColumn.filter(folded),
  };
}

export interface TrailEntry {
  session: SessionView;
  n: number | null;
}

export function trailOf(task: BoardTask): TrailEntry[] {
  const total = new Map<string, number>();
  for (const m of task.members) total.set(m.bot_id, (total.get(m.bot_id) ?? 0) + 1);
  const seen = new Map<string, number>();
  return task.members.map((session) => {
    const n = (seen.get(session.bot_id) ?? 0) + 1;
    seen.set(session.bot_id, n);
    return { session, n: total.get(session.bot_id)! > 1 ? n : null };
  });
}

export function trailView(
  entries: TrailEntry[],
  holderId: string,
  max = TRAIL_MAX,
): { shown: TrailEntry[]; more: number } {
  const shown = entries.slice(0, max);
  const at = entries.findIndex((e) => e.session.id === holderId);
  if (at >= max) shown[max - 1] = entries[at];
  return { shown, more: entries.length - shown.length };
}

export type Step =
  { kind: "screen"; text: string } | { kind: "sent"; from: SessionView; recipient: string; text: string };

export function latestStep(
  task: BoardTask,
  activity: Record<string, SessionActivity>,
  glances: Record<string, Glance>,
): Step | null {
  const text = glances[task.holder.id]?.step ?? null;
  if (text !== null) return { kind: "screen", text };
  let best: Step | null = null;
  let bestAt = "";
  for (const from of task.members) {
    const sent = activity[from.id]?.last_sent;
    if (sent && (best === null || sent.at > bestAt)) {
      best = { kind: "sent", from, recipient: sent.recipient, text: sent.text };
      bestAt = sent.at;
    }
  }
  return best;
}

export function activityTargets(event: UiEvent): string[] {
  switch (event.type) {
    case "message_added":
    case "message_updated":
      return [event.message.sender_session, event.message.recipient_session].filter((id): id is string => id !== null);
    case "session_status":
      return [event.session_id];
    case "session_added":
    case "session_updated":
      return [event.session.id];
    case "workflow_run_changed":
      return event.session_ids;
    default:
      return [];
  }
}

export function glanceTargets(sessions: SessionView[]): string[] {
  return sessions
    .filter((s) => !isArchived(s) && (s.status === "working" || s.status === "idle" || s.status === "needs_attention"))
    .map((s) => s.id);
}

export function noteAttention(
  prev: Record<string, string>,
  sessions: SessionView[],
  now: string,
): Record<string, string> {
  const next: Record<string, string> = {};
  for (const s of sessions) {
    if (!isArchived(s) && s.status === "needs_attention") next[s.id] = prev[s.id] ?? now;
  }
  const keys = Object.keys(next);
  const same = keys.length === Object.keys(prev).length && keys.every((k) => prev[k] === next[k]);
  return same ? prev : next;
}

export function elapsed(since: string, now: Date): { unit: "now" | "min" | "h" | "d"; n: number } {
  const minutes = Math.floor((now.getTime() - new Date(since).getTime()) / 60_000);
  if (minutes < 1) return { unit: "now", n: 0 };
  if (minutes < 60) return { unit: "min", n: minutes };
  if (minutes < 1440) return { unit: "h", n: Math.floor(minutes / 60) };
  return { unit: "d", n: Math.floor(minutes / 1440) };
}
