import { PageHeader } from "./ui";
import { useProjectApi } from "../projectContext";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { type BotView, type SessionView, type Task } from "../api";
import { lastRunText, nextRunText, scheduleText, skipText, sortTasks } from "../tasks";

interface Props {
  tasks: Task[];
  bots: BotView[];
  sessions: SessionView[];
  onNew: () => void;
  onEdit: (taskId: string) => void;
  onOpenSession: (sessionId: string) => void;
  onError: (error: unknown) => void;
}

export default function TasksView({ tasks, bots, sessions, onNew, onEdit, onOpenSession, onError }: Props) {
  const api = useProjectApi();
  const { t, i18n } = useTranslation();
  const [running, setRunning] = useState<string[]>([]);

  async function runNow(task: Task) {
    setRunning((ids) => [...ids, task.id]);
    try {
      await api.runTaskNow(task.id);
    } catch (e) {
      onError(e);
    } finally {
      setRunning((ids) => ids.filter((id) => id !== task.id));
    }
  }

  async function remove(task: Task) {
    const confirmed = await ask(t("tasks.deleteConfirm", { name: task.name }), {
      title: t("tasks.deleteTitle"),
      kind: "warning",
      okLabel: t("tasks.delete"),
      cancelLabel: t("taskForm.cancel"),
    });
    if (!confirmed) return;
    try {
      await api.deleteTask(task.id);
    } catch (e) {
      onError(e);
    }
  }

  return (
    <div className="page resource-page">
      <PageHeader title={t("tasks.title")} description={t("tasks.intro")} actions={<>
        <button className="primary" onClick={onNew}>
          {t("tasks.new")}
        </button>
      </>} />
      {tasks.length === 0 && <p className="empty-state">{t("tasks.empty")}</p>}
      <ul className="resource-list">
        {sortTasks(tasks).map((task) => {
          const bot = bots.find((b) => b.id === task.bot_id);
          const last = lastRunText(t, task, sessions, i18n.language);
          const skip = skipText(t, task, i18n.language);
          return (
            <li key={task.id} className={task.enabled ? "task-row" : "task-row off"}>
              <div className="resource-heading">
                <strong>{task.name}</strong>
                {bot && (
                  <span className="task-bot">
                    <span className="status-dot" style={{ background: bot.color }} />
                    <span className="muted">@{bot.handle}</span>
                  </span>
                )}
                <label className="task-switch">
                  <input
                    type="checkbox"
                    checked={task.enabled}
                    aria-label={t("tasks.switch", { name: task.name })}
                    onChange={(e) => void api.setTaskEnabled(task.id, e.target.checked).catch(onError)}
                  />
                  <span>{t(task.enabled ? "tasks.on" : "tasks.off")}</span>
                </label>
              </div>
              <span className="muted">{scheduleText(t, task, i18n.language)}</span>
              <span className="muted">{t("tasks.next", { when: nextRunText(t, task, i18n.language) })}</span>
              <span className="muted">
                {t("tasks.last")}{" "}
                {last.sessionId ? (
                  <button className="link-button" onClick={() => onOpenSession(last.sessionId!)}>
                    {last.text}
                  </button>
                ) : (
                  last.text
                )}
              </span>
              {skip && <span className="task-skip">{skip}</span>}
              <div className="actions start">
                <button disabled={running.includes(task.id)} onClick={() => void runNow(task)}>
                  {running.includes(task.id) ? t("tasks.running") : t("tasks.runNow")}
                </button>
                <button onClick={() => onEdit(task.id)}>{t("tasks.edit")}</button>
                <button className="danger" onClick={() => void remove(task)}>
                  {t("tasks.delete")}
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
