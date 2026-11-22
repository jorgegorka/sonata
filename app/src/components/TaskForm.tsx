import { useProjectApi } from "../projectContext";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { type BotView, type Task } from "../api";
import { errorMessage } from "../errors";
import { defaultStart, formErrors, type TaskFormInput } from "../tasks";

interface Props {
  task?: Task;
  bots: BotView[];
  onSaved: (task: Task) => void;
  onCancel: () => void;
}

export default function TaskForm({ task, bots, onSaved, onCancel }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [draft, setDraft] = useState<TaskFormInput>(() => {
    const at = task?.at ?? "09:00";
    return {
      name: task?.name ?? "",
      bot_id: task?.bot_id ?? bots[0]?.id ?? "",
      prompt: task?.prompt ?? "",
      every_days: task?.every_days ?? 7,
      at,
      starts_on: defaultStart(task ?? null, new Date(), at),
    };
  });
  const [startPicked, setStartPicked] = useState(false);
  const [shown, setShown] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = <K extends keyof TaskFormInput>(key: K, value: TaskFormInput[K]) => setDraft((d) => ({ ...d, [key]: value }));
  const setAt = (at: string) =>
    setDraft((d) => {
      const follows = !task && !startPicked && !formErrors({ ...d, at }).at;
      return { ...d, at, starts_on: follows ? defaultStart(null, new Date(), at) : d.starts_on };
    });
  const errors = formErrors(draft);
  const problem = (field: keyof TaskFormInput) =>
    shown && errors[field] ? (
      <span className="error-text" role="alert">
        {t(errors[field]!)}
      </span>
    ) : null;

  async function submit(e: FormEvent) {
    e.preventDefault();
    setShown(true);
    setError(null);
    if (Object.keys(errors).length > 0) return;
    setBusy(true);
    try {
      const input = { ...draft, name: draft.name.trim(), starts_on: draft.starts_on };
      onSaved(task ? await api.updateTask(task.id, input) : await api.createTask(input));
    } catch (err) {
      setError(errorMessage(t, err));
      setBusy(false);
    }
  }

  return (
    <form className="create-bot" onSubmit={submit} noValidate>
      <h1>{task ? t("taskForm.editTitle", { name: task.name }) : t("taskForm.createTitle")}</h1>
      <label>
        {t("taskForm.name")}
        <input
          value={draft.name}
          onChange={(e) => set("name", e.target.value)}
          placeholder={t("taskForm.namePlaceholder")}
          autoFocus
        />
        {problem("name")}
      </label>
      <label>
        {t("taskForm.bot")}
        <select value={draft.bot_id} onChange={(e) => set("bot_id", e.target.value)}>
          {bots.length === 0 && <option value="">{t("taskForm.noBots")}</option>}
          {bots.map((b) => (
            <option key={b.id} value={b.id}>
              {b.display_name} (@{b.handle})
            </option>
          ))}
        </select>
        {problem("bot_id")}
      </label>
      <label>
        {t("taskForm.prompt")}
        <textarea value={draft.prompt} onChange={(e) => set("prompt", e.target.value)} rows={8} />
        <span className="muted">{t("taskForm.promptHint")}</span>
        {problem("prompt")}
      </label>
      <div className="task-when">
        <label>
          {t("taskForm.every")}
          <input
            type="number"
            min={1}
            max={365}
            value={Number.isNaN(draft.every_days) ? "" : draft.every_days}
            onChange={(e) => set("every_days", e.target.value === "" ? Number.NaN : Number(e.target.value))}
          />
          <span className="muted">{t("taskForm.days")}</span>
        </label>
        <label>
          {t("taskForm.at")}
          <input type="time" value={draft.at} onChange={(e) => setAt(e.target.value)} />
        </label>
        <label>
          {t("taskForm.startingOn")}
          <input
            type="date"
            value={draft.starts_on}
            onChange={(e) => {
              setStartPicked(true);
              set("starts_on", e.target.value);
            }}
          />
        </label>
      </div>
      {problem("every_days")}
      {problem("at")}
      {problem("starts_on")}
      {error && (
        <div className="error" role="alert">
          {error}
        </div>
      )}
      <div className="actions">
        <button type="button" onClick={onCancel}>
          {t("taskForm.cancel")}
        </button>
        <button type="submit" className="primary" disabled={busy || bots.length === 0}>
          {busy ? t(task ? "taskForm.saving" : "taskForm.creating") : t(task ? "taskForm.save" : "taskForm.create")}
        </button>
      </div>
    </form>
  );
}
