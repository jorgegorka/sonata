import { useProjectApi } from "../projectContext";
import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { api, type BotView, type Glance, type Orchestration, type SessionActivity, type SessionView, type UsageLimit } from "../api";
import { columnTasks, hasWorkGoingOn, type BoardTask, type Column } from "../board";
import { buildOrchestrationBoard } from "../orchestration";
import OrchestrationTaskCard, { OrchestrationTaskDetails } from "./OrchestrationCard";
import WorkBoard from "./WorkBoard";
import { useWorkItems } from "./useWorkItems";
import { useOrchestrations } from "./useOrchestrations";
import { sessionLabel, type Translate } from "../state";
import TaskCard, { type Tone } from "./TaskCard";
import { useBoardData, type BoardData } from "./useBoardData";

export interface Props {
  bots: BotView[];
  sessions: SessionView[];
  paused: boolean;
  limits: UsageLimit[];
  onTogglePause: () => void;
  onOpen: (task: BoardTask) => void;
  onNewTask: () => void;
  onError: (e: unknown) => void;
  drawer?: (tasks: BoardTask[], now: Date, data: BoardData, context?: (task: BoardTask) => ReactNode) => ReactNode;
  data?: BoardData;
  now?: Date;
  orchestrations?: Orchestration[];
}

const COLUMNS: { column: Column; label: string; tone: Tone }[] = [
  { column: "running", label: "board.running", tone: "running" },
  { column: "needs_you", label: "board.needsYou", tone: "waits" },
  { column: "finished", label: "board.finished", tone: "done" },
  { column: "done", label: "board.doneColumn", tone: "done" },
];

const CLOCK_MS = 30_000;

export async function markDone(
  task: BoardTask,
  activity: Record<string, SessionActivity>,
  glances: Record<string, Glance>,
  name: string,
  t: Translate,
): Promise<void> {
  if (task.workItem) { await api.archiveWorkItem(task.workItem.id, true); return; }
  if (task.orchestration?.taskKey) {
    await api.archiveOrchestrationTask(task.orchestration.run.id, task.orchestration.taskKey, true);
    return;
  }
  if (task.orchestration?.run.status === "active") return;
  if (hasWorkGoingOn(task, activity, glances)) {
    const confirmed = await ask(t("board.markDoneConfirm", { name }), {
      title: t("board.markDone"),
      kind: "warning",
      okLabel: t("board.markDone"),
      cancelLabel: t("createBot.cancel"),
    });
    if (!confirmed) return;
  }
  await api.archiveSession(task.id);
}

function useClock(fixed: Date | undefined): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    if (fixed) return;
    const timer = setInterval(() => setNow(new Date()), CLOCK_MS);
    return () => clearInterval(timer);
  }, [fixed]);
  return fixed ?? now;
}

export default function Board({
  bots,
  sessions,
  paused,
  limits,
  onTogglePause,
  onOpen,
  onNewTask,
  onError,
  drawer,
  data,
  now,
  orchestrations,
}: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [showDone, setShowDone] = useState(false);
  const loadedRuns = useOrchestrations(onError);
  const workItems = useWorkItems(onError);
  const runs = orchestrations ?? loadedRuns;
  const [selectedTask, setSelectedTask] = useState<string | null>(null);
  const live = useBoardData(sessions, onError, showDone);
  const shown = data ?? live;
  const { activity, glances, attentionSince } = shown;
  const clock = useClock(now);
  const [clearing, setClearing] = useState(false);
  const providers = useMemo(() => Object.fromEntries(bots.map((b) => [b.id, b.provider])), [bots]);
  const grouped = useMemo(
    () => buildOrchestrationBoard({ sessions, activity, glances, attentionSince, paused, limits, providers }, runs),
    [sessions, activity, glances, attentionSince, paused, limits, providers, runs],
  );
  const tasks = grouped.normal;
  const selected = grouped.children.find((c) => c.id === selectedTask);
  const open = (task: BoardTask) => { setSelectedTask(null); onOpen(task); };
  const openParent = (run: Orchestration) => {
    const parent = tasks.find((task) => task.id === run.session_id);
    if (parent) open(parent);
  };
  const columns = COLUMNS.filter((c) => showDone || c.column !== "done")
    .map((c) => ({ ...c, ...columnTasks(tasks, c.column, clock), children: grouped.children.filter((task) => task.column === c.column)
      .sort((a, b) => (b.task.archived_at ?? b.run.ended_at ?? b.run.created_at).localeCompare(a.task.archived_at ?? a.run.ended_at ?? a.run.created_at)) }));

  async function markAllDone(older: BoardTask[]) {
    const confirmed = await ask(t("board.markAllDoneConfirm", { count: older.length }), {
      title: t("board.markAllDone"),
      kind: "warning",
      okLabel: t("board.markAllDone"),
      cancelLabel: t("createBot.cancel"),
    }).catch((e: unknown) => {
      onError(e);
      return false;
    });
    if (!confirmed) return;
    setClearing(true);
    for (const task of older) {
      await api.archiveSession(task.id).catch(onError);
    }
    setClearing(false);
  }

  const card = (task: BoardTask) => (
    <TaskCard
      key={task.id}
      task={task}
      bots={bots}
      sessions={sessions}
      activity={activity}
      glances={glances}
      now={clock}
      onOpen={() => open(task)}
      onOrchestrationTask={(key) => setSelectedTask(`${task.orchestration!.run.id}:${key}`)}
      onMarkDone={() =>
        void markDone(task, activity, glances, sessionLabel(task.root, sessions, bots, t), t).catch(onError)
      }
      onError={onError}
    />
  );

  if (workItems !== null && orchestrations === undefined) return <WorkBoard items={workItems} bots={bots} sessions={sessions} paused={paused} limits={limits} onTogglePause={onTogglePause} onOpen={onOpen} onNewTask={onNewTask} onError={onError} drawer={drawer} data={shown} now={clock} />;

  return (
    <div className="board">
      <header className="board-header">
        <div className="board-heading">
          <h1>{t("board.title")}</h1>
          <ul className="board-counts">
            {columns.map((c) => (
              <li key={c.column}>
                <span className={`state-dot ${c.tone}`} aria-hidden="true" />
                {`${t(c.label)} ${c.shown.length + c.children.length}`}
              </li>
            ))}
          </ul>
        </div>
        <div className="header-actions">
          <button aria-expanded={showDone} aria-controls="board-done-column" onClick={() => setShowDone((shown) => !shown)}>
            {t(showDone ? "board.hideDone" : "board.showDone")}
          </button>
          <button onClick={onTogglePause}>{paused ? t("board.resume") : t("board.pause")}</button>
          <button className="primary" onClick={onNewTask}>
            {t("board.newTask")}
          </button>
        </div>
      </header>
      {paused && <p className="board-note muted">{t("board.pausedNote")}</p>}
      <div className={showDone ? "board-columns with-done" : "board-columns"}>
        {columns.map((c) => (
          <section
            key={c.column}
            id={`board-${c.column}-column`}
            className={`board-column ${c.column}`}
            data-column={c.column}
            aria-labelledby={`board-${c.column}`}
          >
            <div className="board-column-head">
              <span className={`state-dot ${c.tone}`} aria-hidden="true" />
              <h2 id={`board-${c.column}`}>{t(c.label)}</h2>
              <span className="board-count">{c.shown.length + c.children.length}</span>
            </div>
            {c.shown.length === 0 && c.older.length === 0 && c.children.length === 0 && (
              <div className="board-empty">
                {tasks.length === 0 && c.column === "running" ? (
                  <>
                    <span>{t("board.empty")}</span>
                    <button onClick={onNewTask}>{t("board.newTask")}</button>
                  </>
                ) : (
                  <span>{t("board.emptyColumn")}</span>
                )}
              </div>
            )}
            {c.shown.map(card)}
            {c.children.map((child) => <OrchestrationTaskCard key={child.id} card={child}
              onOpen={() => setSelectedTask(child.id)} onParent={() => openParent(child.run)} onError={onError} />)}
            {c.older.length > 0 && (
              <details className="board-fold">
                <summary>{t("board.older", { n: c.older.length })}</summary>
                <div className="board-fold-body">
                  {c.older.map(card)}
                  <button disabled={clearing} onClick={() => void markAllDone(c.older)}>
                    {t("board.markAllDone")}
                  </button>
                </div>
              </details>
            )}
          </section>
        ))}
      </div>
      {selected ? <OrchestrationTaskDetails card={selected} onClose={() => setSelectedTask(null)}
        onDependency={(key) => setSelectedTask(`${selected.run.id}:${key}`)} onError={onError}
        onExecution={() => { if (selected.execution) open(selected.execution); }} />
        : drawer?.([...tasks, ...grouped.children.flatMap((c) => c.execution ? [c.execution] : [])], clock, shown)}
    </div>
  );
}
