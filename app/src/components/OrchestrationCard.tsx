import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import type { Orchestration } from "../api";
import { isTaskTerminal, orchestrationProgress, type OrchestrationCard as Card } from "../orchestration";
import { useProjectApi } from "../projectContext";
import { useAct } from "./useAct";
import { MessageLink } from "./MessageItem";
import remarkGfm from "remark-gfm";
import { opensPrograms } from "../files";

export function OrchestrationSummary({ run, onTask, onError }: { run: Orchestration; onTask?: (key: string) => void; onError: (e: unknown) => void }) {
  const { t } = useTranslation();
  const api = useProjectApi();
  const { busy, act } = useAct(onError);
  return <div className="orchestration-summary">
    <p>{t("orchestration.progress", orchestrationProgress(run))}</p>
    <p className="muted">{t("orchestration.breakdown", orchestrationProgress(run))}</p>
    <ul>{run.tasks.map((task) => <li key={task.spec.key}>
      <button className="link-button" onClick={() => onTask?.(task.spec.key)} disabled={!onTask}>{task.spec.title}</button>
      <span className="muted"> · {t(`orchestration.state.${task.state}`)}</span>
    </li>)}</ul>
    {run.status === "active" && <button disabled={busy} onClick={() => act(() => api.cancelOrchestration(run.id))}>{t("orchestration.cancelProcess")}</button>}
  </div>;
}

interface Props { card: Card; onOpen: () => void; onParent: () => void; onError: (e: unknown) => void }

export default function OrchestrationTaskCard({ card, onOpen, onParent, onError }: Props) {
  const { t } = useTranslation();
  const api = useProjectApi();
  const { busy, act } = useAct(onError);
  const { task, run } = card;
  const executor = task.spec.executor;
  const runtimeKind = task.state === "running" ? card.execution?.kind : undefined;
  const stateLabel = runtimeKind && ["starting", "queued", "paused", "limited", "background"].includes(runtimeKind)
    ? t(`board.${runtimeKind}`) : t(`orchestration.state.${task.state}`);
  return <article className={`task-card ${card.column}`} data-orchestration-task={task.spec.key}>
    <div className="task-card-head"><button className="task-title" onClick={onOpen}>{task.spec.title}</button></div>
    <div className="task-meta"><span className="task-chip">{executor.kind === "workflow" ? executor.name : `@${executor.handle}`}</span>
      <span className={`task-state ${task.state === "failed" || task.state === "blocked" ? "blocked" : ""}`}>
        {card.column === "needs_you" ? t("orchestration.needsYou") : stateLabel}
      </span></div>
    <button className="link-button orchestration-parent" onClick={onParent}>{t("orchestration.parent", { name: run.title })}</button>
    {task.spec.depends_on.length > 0 && <p className="muted">{t("orchestration.dependencies", { names: task.spec.depends_on.map((key) => run.tasks.find((t) => t.spec.key === key)?.spec.title ?? key).join(", ") })}</p>}
    {task.review && <p className="task-request">{task.review}</p>}
    <div className="task-actions"><button onClick={onOpen}>{t("orchestration.details")}</button>
      {run.status === "active" && !isTaskTerminal(task) && <button disabled={busy} onClick={() => act(() => api.cancelOrchestration(run.id, task.spec.key))}>{t("orchestration.cancelTask")}</button>}
      {isTaskTerminal(task) && <button disabled={busy} onClick={() => act(() => api.archiveOrchestrationTask(run.id, task.spec.key, card.column !== "done"))}>
        {t(card.column === "done" ? "orchestration.restore" : "board.markDone")}</button>}
    </div>
  </article>;
}

export function OrchestrationTaskDetails({ card, onClose, onExecution, onDependency, onError }: {
  card: Card; onClose: () => void; onExecution: () => void; onDependency: (key: string) => void; onError: (e: unknown) => void;
}) {
  const { t } = useTranslation();
  const api = useProjectApi();
  const { busy, act } = useAct(onError);
  const { task, run } = card;
  return <aside className="drawer orchestration-details" aria-label={task.spec.title}>
    <header className="drawer-header"><h2>{task.spec.title}</h2><button onClick={onClose}>{t("board.close")}</button></header>
    <div className="drawer-body orchestration-detail-body markdown">
      <p>{t(`orchestration.state.${task.state}`)}</p>
      <p className="muted">{task.spec.executor.kind === "workflow" ? task.spec.executor.name : `@${task.spec.executor.handle}`}</p>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ a: MessageLink }}>{task.spec.instructions}</ReactMarkdown>
      <h3>{t("orchestration.criteria")}</h3><ul>{task.spec.acceptance_criteria.map((c, i) => <li key={i}>{c}</li>)}</ul>
      {task.spec.depends_on.length > 0 && <><h3>{t("orchestration.prerequisites")}</h3><ul>{task.spec.depends_on.map((key) => <li key={key}>
        <button className="link-button" onClick={() => onDependency(key)}>{run.tasks.find((t) => t.spec.key === key)?.spec.title ?? key}</button>
      </li>)}</ul></>}
      {task.result && <><h3>{t("orchestration.result")}</h3><ReactMarkdown remarkPlugins={[remarkGfm]} components={{ a: MessageLink }}>{task.result}</ReactMarkdown></>}
      {task.review && <><h3>{t("orchestration.review")}</h3><p>{task.review}</p></>}
      {task.attachments.length > 0 && <ul className="attachments">{task.attachments.map((file, i) => <li key={file.path} className="attachment">
        <span className="attachment-name">{file.name}</span>
        {task.result_message_id && <button onClick={() => act(() => api.openAttachment(task.result_message_id!, i, opensPrograms(file.name)))}>
          {t(opensPrograms(file.name) ? "message.reveal" : "message.open")}</button>}
      </li>)}</ul>}
      {card.execution && <button className="primary" onClick={onExecution}>{t("orchestration.openExecution")}</button>}
      {run.status === "active" && !isTaskTerminal(task) && <button disabled={busy} onClick={() => act(() => api.cancelOrchestration(run.id, task.spec.key))}>{t("orchestration.cancelTask")}</button>}
    </div>
  </aside>;
}
