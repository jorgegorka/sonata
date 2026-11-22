import type { SessionView, ThreadSummary } from "./api";
import type { BoardTask } from "./board";
import { followsFirstSend, type Selection } from "./selection";
import { hasArchivedEnd, isArchived } from "./state";

export type DrawerShow = Selection | { kind: "handoffs" };
export interface DrawerView {
  root: string | null;
  show: DrawerShow;
  archivedAt?: string;
}

export interface Recipient {
  kind: "bot" | "workflow";
  id: string;
}

export function blankDrawer(recipient: Recipient): DrawerView {
  const { kind, id } = recipient;
  const show: DrawerShow = kind === "bot" ? { kind: "newSession", botId: id } : { kind: "newRun", workflowId: id };
  return { root: null, show };
}

export function openTask(task: BoardTask): DrawerView {
  const id = task.column === "needs_you" ? task.holder.id : task.id;
  return {
    root: task.id,
    show: { kind: "session", id },
    ...(task.column === "done" ? { archivedAt: task.root.archived_at! } : {}),
  };
}

export function drawerAfter(drawer: DrawerView, sessions: SessionView[], threads: ThreadSummary[]): DrawerView | null {
  const { root, show } = drawer;
  if (root === null) return drawer;
  const archived = drawer.archivedAt !== undefined;
  const live = (id: string) => sessions.some((s) => s.id === id &&
    (archived ? s.archived_at === drawer.archivedAt : !isArchived(s)));
  if (!live(root)) return null;
  let gone = false;
  if (show.kind === "session") gone = !live(show.id);
  if (show.kind === "thread") {
    const thread = threads.find((th) => th.id === show.id);
    gone = thread === undefined || (!archived && hasArchivedEnd(thread, sessions));
  }
  return gone ? { ...drawer, show: { kind: "session", id: root } } : drawer;
}

export function followsInDrawer(drawer: DrawerView | null, created: SessionView): boolean {
  return drawer !== null && drawer.root === null && followsFirstSend(drawer.show, created);
}

export function followsRunStart(drawer: DrawerView | null, workflowId: string): boolean {
  const show = drawer?.root === null ? drawer.show : null;
  return show?.kind === "newRun" && show.workflowId === workflowId;
}

export function handoffsOf(members: SessionView[], threads: ThreadSummary[]): ThreadSummary[] {
  const ids = new Set(members.map((m) => m.id));
  const member = (id: string | null) => id !== null && ids.has(id);
  return threads.filter((th) => member(th.session_a) || member(th.session_b));
}

function onBoard(page: { kind: string } | null, drawer: DrawerView | null): DrawerView | null {
  return page?.kind === "board" ? drawer : null;
}

export function chatShown(page: { kind: string; id?: string } | null, drawer: DrawerView | null): string | null {
  if (page?.kind === "session") return page.id ?? null;
  const show = onBoard(page, drawer)?.show;
  return show?.kind === "session" ? show.id : null;
}

export function threadShown(page: { kind: string; id?: string } | null, drawer: DrawerView | null): string | null {
  if (page?.kind === "thread") return page.id ?? null;
  const show = onBoard(page, drawer)?.show;
  return show?.kind === "thread" ? show.id : null;
}

export function fullChatOf(drawer: DrawerView): Selection | null {
  const { show } = drawer;
  return show.kind === "handoffs" || show.kind === "newRun" ? null : show;
}
