import { useEffect, useState, type ReactNode } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import type { WorkItem } from "../api";
import { FOLD_DAYS, type BoardTask } from "../board";
import { pendingReason, taskExecution, taskProgress } from "../workItems";
import { useProjectApi } from "../projectContext";
import { useAct } from "./useAct";
import TaskCard from "./TaskCard";
import type { Props as BoardProps } from "./Board";
import WorkItemForm from "./WorkItemForm";
import { useBoardData } from "./useBoardData";
import { errorMessage } from "../errors";

export type Props = BoardProps & {
  items: WorkItem[];
  loading?: boolean;
  loadError?: unknown;
  onRetry?: () => void;
  onSaved: (item: WorkItem) => void;
};
export default function WorkBoard(props: Props) {
  const { items, bots, sessions, data, onError } = props;
  const api = useProjectApi(); const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  const [showPending, setShowPending] = useState(false), [showDone, setShowDone] = useState(false);
  const [selected, setSelected] = useState<WorkItem | null>(null);
  const [clock, setClock] = useState(() => new Date());
  useEffect(() => { const timer = setInterval(() => setClock(new Date()), 30_000); return () => clearInterval(timer); }, []);
  const now = props.now ?? clock;
  const live = useBoardData(sessions, onError, showDone);
  const boardData = data ?? live;
  const input = { sessions, ...boardData, paused: props.paused, limits: props.limits,
    providers: Object.fromEntries(bots.map((b) => [b.id, b.provider])) };
  const executions = new Map(items.map((item) => [item.id, taskExecution(item, input)]));
  const status = (item: WorkItem) => item.archived_at ? "done" : item.state === "pending" ? "pending"
    : executions.get(item.id)?.column ?? (item.state === "working" ? "running" : item.state);
  const open = (item: WorkItem) => { props.onCloseCreation(); const execution = executions.get(item.id); setSelected(execution ? null : item); if (execution) props.onOpen(execution); };
  const columns = ["pending", "running", "needs_you", "finished", "done"].filter((c) => (c !== "pending" || showPending) && (c !== "done" || showDone));
  const columnLabels: Record<string, string> = { pending: "work.pending", running: "board.running", needs_you: "board.needsYou", finished: "board.finished", done: "board.doneColumn" };
  const context = (item: WorkItem, detailed = false): ReactNode => {
    const parent = items.find((w) => w.id === item.parent_id);
    const children = items.filter((w) => w.parent_id === item.id);
    const progress = taskProgress(item.id, items);
    return <div className="work-context">
      {parent && <button className="link-button" onClick={() => open(parent)}>{t("work.parent", { title: parent.title })}</button>}
      {item.depends_on.length > 0 && <div><span className="muted">{t("orchestration.prerequisites")}: </span>{item.depends_on.map((id) => {
        const dependency = items.find((w) => w.id === id);
        return dependency && <button key={id} className="link-button work-dependency" onClick={() => open(dependency)}>{dependency.title}</button>;
      })}</div>}
      {progress.total > 0 && <>
        <label className="work-progress">{t("work.progress", progress)}
          <progress max={progress.total} value={progress.finished} aria-label={t("work.progress", progress)} />
        </label>
        {progress.unsuccessful > 0 && <p className="muted">{t("work.unsuccessful", { count: progress.unsuccessful })}</p>}
        <ul className="work-children">{children.map((child) => <li key={child.id}><button className="link-button" onClick={() => open(child)}>{child.title}</button><span className="muted"> · {t(columnLabels[status(child)])}</span></li>)}</ul>
      </>}
      {item.execution?.state === "awaiting_review" && <p className="muted">{t("work.awaitingReview")}</p>}
      {item.outcome && ["failed", "blocked", "cancelled"].includes(item.outcome) && <p className="task-state blocked">{t(`orchestration.state.${item.outcome}`)}</p>}
      {item.execution?.review && <p className="muted">{item.execution.review}</p>}
      {detailed && item.execution && <details><summary>{t("orchestration.details")}</summary>
        <p className="work-instructions">{item.instructions}</p>
        <h3>{t("orchestration.criteria")}</h3><ul>{item.execution.spec.acceptance_criteria.map((criterion, i) => <li key={i}>{criterion}</li>)}</ul>
        {item.execution.result && <><h3>{t("orchestration.result")}</h3><p className="work-instructions">{item.execution.result}</p></>}
      </details>}
      <div className="task-actions">
        {item.state === "pending" && item.manual_start && <button disabled={busy} onClick={() => act(() => api.startWorkItem(item.id))}>{t("work.start")}</button>}
        {!item.archived_at && <button onClick={() => props.onCreate(item)}>{t("work.addSubtask")}</button>}
        {["pending", "working", "needs_you"].includes(item.state) && <button disabled={busy} onClick={() => act(() => api.cancelWorkItem(item.id))}>{t("orchestration.cancelTask")}</button>}
      </div>
    </div>;
  };
  const card = (item: WorkItem) => {
    const execution = executions.get(item.id);
    if (execution) return <TaskCard key={item.id} task={execution} bots={bots} sessions={sessions} activity={boardData.activity} glances={boardData.glances} now={now} onOpen={() => open(item)} onMarkDone={() => act(() => api.archiveWorkItem(item.id, true))} onError={onError}>{context(item)}</TaskCard>;
    return <article className={`task-card ${status(item)}`} key={item.id}>
      <div className="task-card-head"><button className="task-title" onClick={() => open(item)}>{item.title}</button></div>
      <span className="task-state">{item.state === "pending" ? t(`work.${pendingReason(item, items)}`) : t(columnLabels[status(item)])}</span>
      {context(item)}
      {item.state === "finished" && <button disabled={busy} onClick={() => act(() => api.archiveWorkItem(item.id, true))}>{t("board.markDone")}</button>}
      {item.archived_at && <button disabled={busy} onClick={() => act(() => api.archiveWorkItem(item.id, false))}>{t("archive.restore")}</button>}
    </article>;
  };
  return <div className="board">
    <header className="board-header"><h1>{t("board.title")}</h1><div className="header-actions">
      <button aria-expanded={showPending} aria-controls="work-pending" onClick={() => setShowPending(!showPending)}>{t(showPending ? "work.hidePending" : "work.showPending", { count: items.filter((w) => status(w) === "pending").length })}</button>
      <button aria-expanded={showDone} aria-controls="work-done" onClick={() => setShowDone(!showDone)}>{t(showDone ? "board.hideDone" : "board.showDone")}</button>
      <button onClick={props.onTogglePause}>{t(props.paused ? "board.resume" : "board.pause")}</button>
      <button className="primary" onClick={() => props.onCreate()}>{t("board.newTask")}</button>
    </div></header>
    {props.paused && <p className="board-note muted">{t("board.pausedNote")}</p>}
    {props.loading && <p role="status">{t("work.loading")}</p>}
    {props.loadError != null && <div className="error" role="alert">{t("work.loadFailed")} {errorMessage(t, props.loadError)} <button onClick={props.onRetry}>{t("work.retry")}</button></div>}
    <div className="board-columns work-columns" style={{ "--work-columns": columns.length } as React.CSSProperties}>
      {columns.map((column) => {
        const rows = items.filter((w) => status(w) === column).sort((a, b) => column === "done" || column === "finished" ? (b.archived_at ?? b.finished_at ?? b.created_at).localeCompare(a.archived_at ?? a.finished_at ?? a.created_at) : a.created_at.localeCompare(b.created_at));
        const older = column === "finished" ? rows.filter((w) => !executions.get(w.id)?.unread && new Date(w.finished_at ?? w.created_at).getTime() < now.getTime() - FOLD_DAYS * 86_400_000) : [];
        const shown = rows.filter((w) => !older.includes(w));
        return <section id={`work-${column}`} className={`board-column ${column}`} data-column={column} key={column} aria-label={t(columnLabels[column])}>
          <div className="board-column-head"><h2>{t(columnLabels[column])}</h2><span className="board-count">{rows.length}</span></div>
          {rows.length ? shown.map(card) : <div className="board-empty">{!props.loading && props.loadError == null ? t("board.emptyColumn") : ""}</div>}
          {older.length > 0 && <details className="board-fold"><summary>{t("board.older", { n: older.length })}</summary><div className="board-fold-body">{older.map(card)}
            <button disabled={busy} onClick={() => act(async () => {
              if (await ask(t("board.markAllDoneConfirm", { count: older.length }), { title: t("board.markAllDone"), kind: "warning" })) {
                for (const item of older) await api.archiveWorkItem(item.id, true);
              }
            })}>{t("board.markAllDone")}</button>
          </div></details>}
        </section>;
      })}
    </div>
    {props.creation ? <WorkItemForm key={props.creation.id} creation={props.creation} bots={bots}
      siblings={items.filter((w) => w.parent_id === props.creation?.parent?.id)}
      onClose={props.onCloseCreation} onSaved={(item) => { props.onSaved(item); props.onCloseCreation(); }} />
      : selected ? <aside className="drawer" aria-label={selected.title}><header className="drawer-header"><h2>{selected.title}</h2><button onClick={() => setSelected(null)}>{t("board.close")}</button></header><div className="drawer-body work-details">{context(items.find((w) => w.id === selected.id) ?? selected, true)}</div></aside>
      : props.drawer?.([...executions.values()].filter((task): task is BoardTask => task !== null), now, boardData, (task) => task.workItem && <div className="work-details">{context(task.workItem, true)}</div>)}
  </div>;
}
