import type { TFunction } from "i18next";
import type { SessionView, Task, UiEvent } from "./api";

export interface TaskFormInput {
  name: string;
  bot_id: string;
  prompt: string;
  every_days: number;
  at: string;
  starts_on: string;
}

const NAME_MAX = 40;
const EVERY_MIN = 1;
const EVERY_MAX = 365;

const DATE_TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

function formatted(iso: string, locale: string, options: Intl.DateTimeFormatOptions): string {
  return new Date(iso).toLocaleString(locale, options);
}

function timeOfDay(at: string, locale: string): string {
  const m = /^(\d{2}):(\d{2})$/.exec(at);
  if (!m) return at;
  const local = new Date(2000, 0, 1, Number(m[1]), Number(m[2]));
  return new Intl.DateTimeFormat(locale, { timeStyle: "short" }).format(local);
}

export function scheduleText(t: TFunction, task: Task, locale: string): string {
  return t("tasks.every", { count: task.every_days, time: timeOfDay(task.at, locale) });
}

export function nextRunText(t: TFunction, task: Task, locale: string): string {
  if (!task.enabled || !task.next_run_at) return t("tasks.off");
  return formatted(task.next_run_at, locale, DATE_TIME);
}

export function lastRunText(
  t: TFunction,
  task: Task,
  sessions: SessionView[],
  locale: string,
): { text: string; sessionId: string | null } {
  if (!task.last_run_at) return { text: t("tasks.never"), sessionId: null };
  const id = task.last_run_session;
  if (!id || !sessions.some((s) => s.id === id)) return { text: t("tasks.removed"), sessionId: null };
  return { text: formatted(task.last_run_at, locale, DATE_TIME), sessionId: id };
}

export function skipText(t: TFunction, task: Task, locale: string): string | null {
  if (!task.skipped_at) return null;
  if (task.last_run_at && new Date(task.skipped_at) <= new Date(task.last_run_at)) return null;
  const when = formatted(task.skipped_at, locale, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  return t("tasks.skipped", { when });
}

export function sortTasks(tasks: Task[]): Task[] {
  const next = (task: Task) => (task.enabled && task.next_run_at ? new Date(task.next_run_at).getTime() : Infinity);
  return [...tasks].sort((a, b) => {
    const [x, y] = [next(a), next(b)];
    if (x !== y) return x < y ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

function validDate(date: string): boolean {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!m) return false;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const parsed = new Date(y, mo - 1, d);
  return parsed.getFullYear() === y && parsed.getMonth() === mo - 1 && parsed.getDate() === d;
}

export function formErrors(input: TaskFormInput): Partial<Record<keyof TaskFormInput, string>> {
  const errors: Partial<Record<keyof TaskFormInput, string>> = {};
  const name = [...input.name.trim()].length;
  if (name === 0) errors.name = "taskForm.nameRequired";
  else if (name > NAME_MAX) errors.name = "taskForm.nameTooLong";
  if (!input.bot_id) errors.bot_id = "taskForm.botRequired";
  if (input.prompt.trim() === "") errors.prompt = "taskForm.promptRequired";
  if (!Number.isInteger(input.every_days) || input.every_days < EVERY_MIN || input.every_days > EVERY_MAX) {
    errors.every_days = "taskForm.everyRange";
  }
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(input.at)) errors.at = "taskForm.timeInvalid";
  if (!validDate(input.starts_on)) errors.starts_on = "taskForm.startInvalid";
  return errors;
}

export function reloadsTasks(event: UiEvent): boolean {
  return event.type === "tasks_changed";
}

function localDate(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function defaultStart(task: Task | null, now: Date, at: string): string {
  if (task) return task.next_run_at ? localDate(new Date(task.next_run_at)) : task.starts_on;
  const m = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(at);
  const [y, mo, d] = [now.getFullYear(), now.getMonth(), now.getDate()];
  if (!m || new Date(y, mo, d, Number(m[1]), Number(m[2])) > now) return localDate(new Date(y, mo, d));
  return localDate(new Date(y, mo, d + 1));
}
