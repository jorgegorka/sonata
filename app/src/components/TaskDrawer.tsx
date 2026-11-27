import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  api,
  type BotView,
  type RunGlance,
  type RunView,
  type SessionActivity,
  type SessionView,
  type ThreadSummary,
  type Visit,
} from "../api";
import { isUnread, trailOf, trailView, type BoardTask } from "../board";
import { avatarTextColor } from "../colors";
import { fullChatOf, type DrawerShow, type DrawerView } from "../drawer";
import { handleOf, sessionLabel, threadName } from "../state";
import { runMapState } from "../workflows";
import SessionName from "./SessionName";
import { stateText, toneOf } from "./TaskCard";
import WorkflowMap from "./WorkflowMap";
import WorkflowPause, { StopRun } from "./WorkflowPause";
import RestoreTaskButton from "./RestoreTaskButton";
import { isTaskTerminal } from "../orchestration";

interface Props {
  task: BoardTask | null;
  drawer: DrawerView;
  bots: BotView[];
  sessions: SessionView[];
  activity: Record<string, SessionActivity>;
  now: Date;
  onShow: (show: DrawerShow) => void;
  onOpenFull: () => void;
  onMarkDone: () => void;
  onClose: () => void;
  onError: (e: unknown) => void;
  children: ReactNode;
  taskContext?: ReactNode;
}

export default function TaskDrawer(props: Props) {
  const { task, drawer, bots, sessions, onOpenFull, onMarkDone, onClose, onError, children } = props;
  const { t } = useTranslation();
  if (!task) return null;
  const title = sessionLabel(task.root, sessions, bots, t);
  const process = task?.orchestration;
  const canMarkDone = task?.workItem ? task.workItem.state === "finished" : !process || (process.taskKey ? process.run.tasks.some((t) => t.spec.key === process.taskKey && isTaskTerminal(t)) : process.run.status !== "active");
  return (
    <aside className="drawer" aria-label={title}>
      <header className="drawer-header">
        <div className="drawer-top">
          <h2 className="drawer-title" title={title}>
            <SessionName key={task.root.id} session={task.root} label={title} onError={onError} />
          </h2>
          <button
            className="icon-button drawer-icon"
            disabled={fullChatOf(drawer) === null}
            aria-label={t("board.openFull")}
            title={t("board.openFull")}
            onClick={onOpenFull}
          >
            <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true" className="stroke-glyph">
              <path d="M10 2.5h3.5V6M13.5 2.5L9 7M6 13.5H2.5V10M2.5 13.5L7 9" />
            </svg>
          </button>
          {task && (task.column === "done" ? (
            <RestoreTaskButton workItemId={task.workItem?.id} taskId={task.id} onError={onError} />
          ) : (
            <button disabled={!canMarkDone} onClick={onMarkDone}>{t("board.markDone")}</button>
          ))}
          <button
            className="icon-button drawer-icon"
            aria-label={t("board.close")}
            title={t("board.close")}
            onClick={onClose}
          >
            <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true" className="stroke-glyph">
              <path d="M4 4l8 8M12 4l-8 8" />
            </svg>
          </button>
        </div>
        {task && <TaskLine {...props} task={task} />}
      </header>
      {task?.workflow && (
        <RunPanel
          key={task.workflow.run_id}
          glance={task.workflow}
          bots={bots}
          onShow={props.onShow}
          onError={onError}
          archivedMembers={task.column === "done" ? task.members.map((m) => m.id) : undefined}
        />
      )}
      <div className="drawer-body">{props.taskContext}{children}</div>
    </aside>
  );
}

function TaskLine({ task, drawer, bots, activity, now, onShow }: Props & { task: BoardTask }) {
  const { t } = useTranslation();
  const [allOf, setAllOf] = useState<string | null>(null);
  const entries = trailOf(task);
  const shownId = drawer.show.kind === "session" ? drawer.show.id : null;
  const { shown, more } =
    allOf === task.id ? { shown: entries, more: 0 } : trailView(entries, shownId ?? task.holder.id);
  const handoffs = drawer.show.kind === "handoffs" || drawer.show.kind === "thread";
  const tone = toneOf(task.kind);
  const needing = new Map(task.needing.map((n) => [n.id, n.kind]));
  return (
    <div className="drawer-meta">
      <ol className="trail">
        {shown.map(({ session, n }) => {
          const bot = bots.find((b) => b.id === session.bot_id);
          const handle = handleOf(bots, session.bot_id) ?? t("thread.deletedBot");
          const current = session.id === shownId;
          const needs = needing.get(session.id);
          return (
            <li key={session.id} className="trail-entry">
              <button
                className={current ? "trail-who selected" : "trail-who"}
                aria-current={current ? "true" : undefined}
                onClick={() => onShow({ kind: "session", id: session.id })}
              >
                {bot && (
                  <span
                    className="avatar small"
                    style={{ background: bot.color, color: avatarTextColor(bot.color) }}
                    aria-hidden="true"
                  >
                    {bot.display_name.slice(0, 1).toUpperCase()}
                  </span>
                )}
                <span className="trail-name">{n === null ? handle : `${handle} · ${n}`}</span>
                {needs !== undefined ? (
                  <span
                    className={`state-dot ${toneOf(needs)}`}
                    role="img"
                    aria-label={t("board.memberNeedsYou")}
                    title={t("board.memberNeedsYou")}
                  />
                ) : (
                  session.id === task.holder.id && <span className={`state-dot ${tone}`} aria-hidden="true" />
                )}
                {isUnread([session], activity) && (
                  <span className="unread-dot" role="img" aria-label={t("board.unread")} title={t("board.unread")} />
                )}
              </button>
            </li>
          );
        })}
        {more > 0 && (
          <li className="trail-entry trail-more">
            <button className="trail-who" onClick={() => setAllOf(task.id)}>
              {t("board.more", { n: more })}
            </button>
          </li>
        )}
      </ol>
      <button
        className={handoffs ? "drawer-handoffs selected" : "drawer-handoffs"}
        aria-pressed={handoffs}
        onClick={() => onShow({ kind: "handoffs" })}
      >
        {t("board.handoffs")}
      </button>
      <span className={`task-state ${tone}`}>
        <span className={`state-dot ${tone}`} aria-hidden="true" />
        {stateText(task, now, t)}
      </span>
    </div>
  );
}

function RunPanel({
  glance,
  bots,
  onShow,
  onError,
  archivedMembers,
}: {
  glance: RunGlance;
  bots: BotView[];
  onShow: (show: DrawerShow) => void;
  onError: (e: unknown) => void;
  archivedMembers?: string[];
}) {
  const [view, setView] = useState<RunView | null>(null);
  const errorRef = useRef(onError);
  errorRef.current = onError;
  const runId = glance.run_id;
  useEffect(() => {
    let live = true;
    let unlisten: (() => void) | undefined;
    const load = () =>
      api
        .getWorkflowRun(runId)
        .then((run) => {
          if (live) setView(run);
        })
        .catch((e: unknown) => {
          if (live) errorRef.current(e);
        });
    load();
    api
      .onUiEvent((event) => {
        if (event.type === "workflow_run_changed" && event.run_id === runId) load();
      })
      .then((u) => {
        if (live) unlisten = u;
        else u();
      })
      .catch((e: unknown) => errorRef.current(e));
    return () => {
      live = false;
      unlisten?.();
    };
  }, [runId]);
  const state = useMemo(() => view && runMapState(view.run.def, view.visits), [view]);
  const openStep = (stepId: string) => {
    let latest: Visit | null = null;
    for (const v of view?.visits ?? []) {
      if (v.step_id === stepId && v.session_id !== null && (latest === null || v.ord > latest.ord)) latest = v;
    }
    if (latest?.session_id && (archivedMembers === undefined || archivedMembers.includes(latest.session_id))) {
      onShow({ kind: "session", id: latest.session_id });
    }
  };
  const going = glance.status === "paused" || glance.status === "running";
  if (!going && state === null) return null;
  return (
    <section className="drawer-run">
      {archivedMembers === undefined && glance.status === "paused" && <WorkflowPause glance={glance} onError={onError} />}
      {archivedMembers === undefined && glance.status === "running" && <StopRun glance={glance} onError={onError} />}
      {view && state && (
        <div className="drawer-run-map">
          <WorkflowMap def={view.run.def} bots={bots} state={state} onOpenStep={openStep} />
        </div>
      )}
    </section>
  );
}

export function HandoffList({
  threads,
  bots,
  sessions,
  onOpen,
}: {
  threads: ThreadSummary[];
  bots: BotView[];
  sessions: SessionView[];
  onOpen: (threadId: string) => void;
}) {
  const { t } = useTranslation();
  if (threads.length === 0) return <p className="muted empty drawer-note">{t("board.noHandoffs")}</p>;
  return (
    <ul className="skills-list drawer-list">
      {threads.map((th) => (
        <li key={th.id}>
          <button className="skill-row" onClick={() => onOpen(th.id)}>
            <strong>{threadName(th, sessions, bots, t)}</strong>
          </button>
        </li>
      ))}
    </ul>
  );
}
