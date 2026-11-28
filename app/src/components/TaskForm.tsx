import { useProjectApi } from "../projectContext";
import { useId, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { type BotView, type Task } from "../api";
import { defaultStart, formErrors, type TaskFormInput } from "../tasks";
import { Button, Dialog, DialogHeader, Field } from "./ui";
import { useFormGuard } from "./useFormGuard";

interface Props {
  task?: Task;
  bots: BotView[];
  onSaved: (task: Task) => void;
  onCancel: () => void;
}

export default function TaskForm({ task, bots, onSaved, onCancel }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const id = useId();
  const [draft, setDraft] = useState<TaskFormInput>(() => {
    const at = task?.at ?? "09:00";
    return { name: task?.name ?? "", bot_id: task?.bot_id ?? bots[0]?.id ?? "", prompt: task?.prompt ?? "",
      every_days: task?.every_days ?? 7, at, starts_on: defaultStart(task ?? null, new Date(), at) };
  });
  const initial = useRef(draft);
  const guard = useFormGuard(JSON.stringify(initial.current) !== JSON.stringify(draft), onCancel);
  const [startPicked, setStartPicked] = useState(false);
  const [shown, setShown] = useState(false);
  const set = <K extends keyof TaskFormInput>(key: K, value: TaskFormInput[K]) => setDraft((d) => ({ ...d, [key]: value }));
  const setAt = (at: string) => setDraft((d) => ({ ...d, at,
    starts_on: !task && !startPicked && !formErrors({ ...d, at }).at ? defaultStart(null, new Date(), at) : d.starts_on,
  }));
  const errors = formErrors(draft);
  const problem = (field: keyof TaskFormInput) => shown && errors[field]
    ? <span className="error-text" role="alert">{t(errors[field]!)}</span> : null;
  function submit(event: FormEvent) {
    event.preventDefault();
    if (guard.locked) return;
    setShown(true);
    if (Object.keys(errors).length) return;
    const input = { ...draft, name: draft.name.trim() };
    void guard.save(() => task ? api.updateTask(task.id, input) : api.createTask(input), onSaved);
  }
  return <Dialog labelledBy={`${id}-heading`} onRequestClose={() => void guard.requestClose()} submitOnShortcut>
    <form className="dialog-form" onSubmit={submit} noValidate aria-busy={guard.busy}>
      <DialogHeader id={`${id}-heading`} icon="schedule" title={task ? t("taskForm.editTitle", { name: task.name }) : t("taskForm.createTitle")} locked={guard.locked} onClose={() => void guard.requestClose()} />
      <div className="dialog-body form-stack">
        <Field label={t("taskForm.name")} className="title-field" error={problem("name")}>
          <input className="editor-title" data-autofocus value={draft.name} disabled={guard.locked} onChange={(e) => set("name", e.target.value)} placeholder={t("taskForm.namePlaceholder")} aria-invalid={shown && !!errors.name} />
        </Field>
        <Field label={t("taskForm.prompt")} hint={t("taskForm.promptHint")} error={problem("prompt")}>
          <textarea className="editor-description" value={draft.prompt} disabled={guard.locked} onChange={(e) => set("prompt", e.target.value)} rows={5} aria-invalid={shown && !!errors.prompt} />
        </Field>
        <Field label={t("taskForm.bot")} error={problem("bot_id")}>
          <select value={draft.bot_id} disabled={guard.locked} onChange={(e) => set("bot_id", e.target.value)} aria-invalid={shown && !!errors.bot_id}>
            {bots.length === 0 && <option value="">{t("taskForm.noBots")}</option>}
            {bots.map((bot) => <option key={bot.id} value={bot.id}>{bot.display_name} (@{bot.handle})</option>)}
          </select>
        </Field>
        <div className="task-when">
          <Field label={t("taskForm.every")} hint={t("taskForm.days")} error={problem("every_days")}>
            <input type="number" min={1} max={365} value={Number.isNaN(draft.every_days) ? "" : draft.every_days} disabled={guard.locked} onChange={(e) => set("every_days", e.target.value === "" ? Number.NaN : Number(e.target.value))} aria-invalid={shown && !!errors.every_days} />
          </Field>
          <Field label={t("taskForm.at")} error={problem("at")}><input type="time" value={draft.at} disabled={guard.locked} onChange={(e) => setAt(e.target.value)} aria-invalid={shown && !!errors.at} /></Field>
          <Field label={t("taskForm.startingOn")} error={problem("starts_on")}><input type="date" value={draft.starts_on} disabled={guard.locked} onChange={(e) => { setStartPicked(true); set("starts_on", e.target.value); }} aria-invalid={shown && !!errors.starts_on} /></Field>
        </div>
        {guard.error && <div className="error" role="alert">{guard.error}</div>}
      </div>
      <footer className="dialog-footer actions">
        <Button onClick={() => void guard.requestClose()} disabled={guard.locked}>{t("taskForm.cancel")}</Button>
        <Button type="submit" variant="primary" disabled={guard.locked || bots.length === 0}>{guard.busy ? t(task ? "taskForm.saving" : "taskForm.creating") : t(task ? "taskForm.save" : "taskForm.create")}</Button>
      </footer>
    </form>
  </Dialog>;
}
