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
  type Workflow,
} from "../api";
import { isUnread, trailOf, trailView, type BoardTask } from "../board";
import { avatarTextColor } from "../colors";
import { fullChatOf, type DrawerShow, type DrawerView, type Recipient } from "../drawer";
import { handleOf, sessionLabel, threadName } from "../state";
import { runMapState } from "../workflows";
import SessionName from "./SessionName";
import { stateText, toneOf } from "./TaskCard";
import WorkflowMap from "./WorkflowMap";
import WorkflowPause, { StopRun } from "./WorkflowPause";

interface Props {
  task: BoardTask | null;
  drawer: DrawerView;
  bots: BotView[];
  workflows: Workflow[];
  sessions: SessionView[];
  activity: Record<string, SessionActivity>;
  now: Date;
  onShow: (show: DrawerShow) => void;
  onPick: (target: Recipient) => void;
  onOpenFull: () => void;
  onMarkDone: () => void;
  onClose: () => void;
  onError: (e: unknown) => void;
  children: ReactNode;
}

export default function TaskDrawer(props: Props) {
  const { task, drawer, bots, sessions, onOpenFull, onMarkDone, onClose, onError, children } = props;
  const { t } = useTranslation();
  const title = task ? sessionLabel(task.root, sessions, bots, t) : t("board.newTask");
  return (
    <aside className="drawer" aria-label={title}>
      <header className="drawer-header">
        <div className="drawer-top">
          <h2 className="drawer-title" title={title}>
            {task ? <SessionName key={task.root.id} session={task.root} label={title} onError={onError} /> : title}
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
          {task && <button onClick={onMarkDone}>{t("board.markDone")}</button>}
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
        {task ? <TaskLine {...props} task={task} /> : <RecipientPicker {...props} />}
      </header>
      {task?.workflow && (
        <RunPanel
          key={task.workflow.run_id}
          glance={task.workflow}
          bots={bots}
          onShow={props.onShow}
          onError={onError}
        />
      )}
      <div className="drawer-body">{children}</div>
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

const pickValue = ({ kind, id }: Recipient) => `${kind}:${id}`;

function pickedOf(value: string): Recipient {
  const at = value.indexOf(":");
  return { kind: value.slice(0, at) === "workflow" ? "workflow" : "bot", id: value.slice(at + 1) };
}

function RecipientPicker({ drawer, bots, workflows, onPick }: Props) {
  const { t } = useTranslation();
  const { show } = drawer;
  const value =
    show.kind === "newSession"
      ? pickValue({ kind: "bot", id: show.botId })
      : show.kind === "newRun"
        ? pickValue({ kind: "workflow", id: show.workflowId })
        : "";
  return (
    <label className="drawer-bot">
      <span className="muted">{t("board.pickBot")}</span>
      <select value={value} onChange={(e) => onPick(pickedOf(e.target.value))}>
        <optgroup label={t("board.pickBots")}>
          {bots.map((b) => (
            <option key={b.id} value={pickValue({ kind: "bot", id: b.id })}>
              {b.display_name}
            </option>
          ))}
        </optgroup>
        {workflows.length > 0 && (
          <optgroup label={t("board.pickWorkflows")}>
            {workflows.map((w) => (
              <option key={w.id} value={pickValue({ kind: "workflow", id: w.id })}>
                {w.name}
              </option>
            ))}
          </optgroup>
        )}
      </select>
    </label>
  );
}

function RunPanel({
  glance,
  bots,
  onShow,
  onError,
}: {
  glance: RunGlance;
  bots: BotView[];
  onShow: (show: DrawerShow) => void;
  onError: (e: unknown) => void;
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
    if (latest?.session_id) onShow({ kind: "session", id: latest.session_id });
  };
  const going = glance.status === "paused" || glance.status === "running";
  if (!going && state === null) return null;
  return (
    <section className="drawer-run">
      {glance.status === "paused" && <WorkflowPause glance={glance} onError={onError} />}
      {glance.status === "running" && <StopRun glance={glance} onError={onError} />}
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
