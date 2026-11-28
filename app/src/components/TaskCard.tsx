import { Icon } from "./ui";
import { useRef, useState, type ReactNode, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { answerBody } from "../answer";
import { api, USER, type BotView, type Glance, type SessionActivity, type SessionView } from "../api";
import { elapsed, latestStep, trailOf, trailView, type BoardTask, type CardKind } from "../board";
import { avatarTextColor } from "../colors";
import { opensPrograms } from "../files";
import i18n from "../i18n";
import { formatReset } from "../limits";
import { handleOf, sessionLabel, type Translate } from "../state";
import { glanceLine } from "../workflows";
import { useAct } from "./useAct";
import WorkflowPause from "./WorkflowPause";
import RestoreTaskButton from "./RestoreTaskButton";
import { OrchestrationSummary } from "./OrchestrationCard";

interface Props {
  children?: ReactNode;
  task: BoardTask;
  bots: BotView[];
  sessions: SessionView[];
  activity: Record<string, SessionActivity>;
  glances: Record<string, Glance>;
  now: Date;
  onOpen: () => void;
  onMarkDone: () => void;
  onError: (e: unknown) => void;
  onOrchestrationTask?: (key: string) => void;
}

export type Tone = "running" | "waits" | "blocked" | "done";

export function toneOf(kind: CardKind): Tone {
  if (kind === "question" || kind === "workflow") return "waits";
  if (kind === "dialog" || kind === "cut_off") return "blocked";
  return kind === "finished" || kind === "done" ? "done" : "running";
}

export function answerTarget(task: BoardTask): { botId: string; sessionId: string } {
  return { botId: task.holder.bot_id, sessionId: task.holder.id };
}

function ago(since: string, now: Date, t: Translate): string {
  const { unit, n } = elapsed(since, now);
  if (unit === "now") return t("board.now");
  return t(unit === "min" ? "board.minutes" : unit === "h" ? "board.hours" : "board.days", { n });
}

const NAMES_RUN = new Set<CardKind>(["starting", "working", "background"]);

export function stateText(task: BoardTask, now: Date, t: Translate): string {
  const run = task.workflow;
  if (run !== null && (task.kind === "workflow" || (run.status === "running" && NAMES_RUN.has(task.kind)))) {
    return glanceLine(run, t);
  }
  const time = task.since === null ? null : ago(task.since, now, t);
  const timed = (key: string, untimed: string) => (time === null ? t(untimed) : t(key, { time }));
  switch (task.kind) {
    case "orchestration":
      return task.orchestration?.taskKey
        ? t(`orchestration.state.${task.orchestration.run.tasks.find((t) => t.spec.key === task.orchestration?.taskKey)?.state ?? "running"}`)
        : t("orchestration.coordinating");
    case "question":
      return timed("board.asks", "sidebar.waitingForYou");
    case "dialog":
    case "cut_off":
      return timed("board.blocked", "status.needs_attention");
    case "working":
      return timed("board.working", "status.working");
    case "finished":
    case "done":
      return timed("board.done", "status.idle");
    case "run_waits":
      if (run?.handle) return t("board.workflowWaits", { handle: run.handle });
      return run === null ? "" : glanceLine(run, t);
    case "limited":
      return task.limit?.resets_at
        ? t("board.limitedUntil", { time: formatReset(task.limit.resets_at, now, i18n.language) })
        : t("board.limited");
    default:
      return t(`board.${task.kind}`);
  }
}

const bare = (bots: BotView[], botId: string) => bots.find((b) => b.id === botId)?.handle ?? botId;

export default function TaskCard(props: Props) {
  const { task, bots, sessions, now, onOpen } = props;
  const { t } = useTranslation();
  const tone = task.kind === "orchestration" && task.column === "needs_you" ? "waits" : toneOf(task.kind);
  return (
    <article className={`task-card ${task.column}`}>
      <div className="task-card-head">
        <button className="task-title" onClick={onOpen}>
          {sessionLabel(task.root, sessions, bots, t)}
        </button>
        {task.recurring !== null && (
          <span className="task-chip" title={task.recurring}>
            {t("board.recurring")}
          </span>
        )}
        {task.unread && (
          <span className="unread-dot" role="img" aria-label={t("board.unread")} title={t("board.unread")} />
        )}
      </div>
      <div className="task-meta">
        <Trail task={task} bots={bots} />
        <span className={`task-state ${tone}`}>
          <span className={`state-dot ${tone}`} aria-hidden="true" />
          {stateText(task, now, t)}
        </span>
      </div>
      {props.children}
      {task.orchestration && !task.orchestration.taskKey && <OrchestrationSummary run={task.orchestration.run}
        onTask={props.onOrchestrationTask} onError={props.onError} />}
      {task.column === "running" && <RunningBody {...props} />}
      {task.kind === "question" && <QuestionBody {...props} />}
      {task.kind === "dialog" && <DialogBody {...props} />}
      {task.kind === "cut_off" && <CutOffBody {...props} />}
      {task.column !== "done" && task.workflow?.status === "paused" && <WorkflowPause glance={task.workflow} onError={props.onError} />}
      {task.needing.length > 1 && (
        <p className="task-more muted">{t("board.moreNeeding", { n: task.needing.length - 1 })}</p>
      )}
      {(task.column === "finished" || task.column === "done") && <FinishedBody {...props} />}
    </article>
  );
}

function Trail({ task, bots }: { task: BoardTask; bots: BotView[] }) {
  const { t } = useTranslation();
  const { shown, more } = trailView(trailOf(task), task.holder.id);
  return (
    <ol className="trail">
      {shown.map(({ session, n }) => {
        const bot = bots.find((b) => b.id === session.bot_id);
        const handle = handleOf(bots, session.bot_id) ?? t("thread.deletedBot");
        const holds = session.id === task.holder.id;
        return (
          <li key={session.id} className="trail-entry">
            <span className={holds ? "trail-who holder" : "trail-who"} aria-current={holds ? "true" : undefined}>
              {bot && (
                <span
                  className="avatar small"
                  style={{ background: bot.color, color: avatarTextColor(bot.color) }}
                  aria-hidden="true"
                >
                  {bot.display_name.slice(0, 1).toUpperCase()}
                </span>
              )}
              {n === null ? handle : `${handle} · ${n}`}
            </span>
          </li>
        );
      })}
      {more > 0 && <li className="trail-entry trail-more">{t("board.more", { n: more })}</li>}
    </ol>
  );
}

function RunningBody({ task, bots, activity, glances, onError }: Props) {
  const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  const provider = bots.find((b) => b.id === task.holder.bot_id)?.provider;
  const request = task.kind === "background" ? "" : (activity[task.holder.id]?.working_on?.text ?? "");
  const step = latestStep(task, activity, glances);
  const handle = (botId: string) => handleOf(bots, botId) ?? t("thread.deletedBot");
  return (
    <>
      {request !== "" && <p className="task-request">{request}</p>}
      {step?.kind === "screen" && (
        <p className="task-step screen">
          <span className="task-step-glyph" aria-hidden="true">
            ▸
          </span>
          <span className="task-step-text">{step.text}</span>
        </p>
      )}
      {step?.kind === "sent" && (
        <p className="task-step">
          <span className="task-step-who">
            {`${handle(step.from.bot_id)} → ${step.recipient === USER ? t("message.you") : handle(step.recipient)}`}
          </span>
          <span className="task-step-text">{step.text}</span>
        </p>
      )}
      {task.kind === "limited" && provider !== undefined && (
        <div className="task-actions">
          <button className="push" disabled={busy} onClick={() => act(() => api.tryUsageLimitNow(provider))}>
            {t("board.tryNow")}
          </button>
        </div>
      )}
    </>
  );
}

function QuestionBody({ task, bots, activity, onError }: Props) {
  const { t } = useTranslation();
  const [answer, setAnswer] = useState("");
  const { busy, act } = useAct(onError);
  const box = useRef<HTMLTextAreaElement>(null);
  const question = activity[task.holder.id]?.question ?? null;
  const placeholder = t("board.answerPlaceholder", { handle: bare(bots, task.holder.bot_id) });

  function send(option?: string) {
    const sent = answer;
    const body = option === undefined ? sent.trim() : answerBody(option, sent);
    if (busy || !body) return;
    box.current?.blur();
    const { botId, sessionId } = answerTarget(task);
    act(() =>
      api.sendMessage(botId, sessionId, body).then(() => {
        setAnswer((current) => {
          const after = current.startsWith(sent) ? current.slice(sent.length).trimStart() : current;
          return after.trim() === "" ? "" : after;
        });
        return api.markSessionSeen(sessionId);
      }),
    );
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    }
  }

  return (
    <>
      {question !== null && <p className="task-question">{question}</p>}
      {task.holder.waiting_options.length > 0 && (
        <div className="answer-options">
          {task.holder.waiting_options.map((option) => (
            <button key={option} disabled={busy} onClick={() => send(option)}>
              {answer.trim() ? t("chat.optionWithComment", { option }) : option}
            </button>
          ))}
        </div>
      )}
      <div className="task-answer">
        <textarea
          ref={box}
          rows={2}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          value={answer}
          onChange={(e) => setAnswer(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          aria-label={placeholder}
        />
        <button className="primary" disabled={busy || !answer.trim()} onClick={() => send()}>
          {t("board.send")}
        </button>
      </div>
    </>
  );
}

function DialogBody({ task, glances, onError }: Props) {
  const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  const id = task.holder.id;
  const tail = glances[id]?.tail ?? null;
  return (
    <>
      {tail !== null && <pre className="task-tail">{tail}</pre>}
      <div className="task-actions">
        <button disabled={busy} onClick={() => act(() => api.answerDialog(id, "continue"))}>
          {t("attention.continue")}
        </button>
        <button disabled={busy} onClick={() => act(() => api.answerDialog(id, "cancel"))}>
          {t("attention.cancel")}
        </button>
        <button className="primary" disabled={busy} onClick={() => act(() => api.restartSession(id))}>
          {t("attention.restart")}
        </button>
      </div>
    </>
  );
}

function CutOffBody({ task, bots, activity, onError }: Props) {
  const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  const cut = activity[task.holder.id]?.cut_off ?? null;
  const handle = bare(bots, task.holder.bot_id);
  return (
    <div className="task-actions">
      <p className="task-what">{t(cut?.sender === USER ? "board.cutOffTurn" : "board.cutOffSent", { handle })}</p>
      {cut && (
        <button className="push" disabled={busy} onClick={() => act(() => api.retryMessage(cut.message_id))}>
          {t("message.retry")}
        </button>
      )}
    </div>
  );
}

function FinishedBody({ task, bots, activity, onOpen, onMarkDone, onError }: Props) {
  const { t } = useTranslation();
  const reply = activity[task.root.id]?.last_reply ?? null;
  return (
    <>
      {reply !== null && reply.raw && <p className="task-reply muted">{t("message.rawOutput")}</p>}
      {reply !== null && !reply.raw && reply.text !== "" && <p className="task-reply">{reply.text}</p>}
      {task.owes !== null && <p className="task-owes">{t("board.owes", { handle: bare(bots, task.owes.bot_id) })}</p>}
      {reply !== null && reply.files.length > 0 && (
        <ul className="task-files">
          {reply.files.map((name, i) => (
            <li key={`${i}-${name}`}>
              <button
                className="chip task-file"
                title={name}
                onClick={() => void api.openAttachment(reply.message_id, i, opensPrograms(name)).catch(onError)}
              >
                <Icon name="attach" /><span>{name}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="task-actions">
        <button className="push" onClick={onOpen}>
          {t("board.open")}
        </button>
        {task.column === "done" ? (
          <RestoreTaskButton workItemId={task.workItem?.id} taskId={task.id} onError={onError} />
        ) : (
          <button onClick={onMarkDone}>{t("board.markDone")}</button>
        )}
      </div>
    </>
  );
}
