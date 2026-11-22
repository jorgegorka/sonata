import i18next, { type TFunction } from "i18next";
import { beforeAll, describe, expect, it } from "vitest";
import de from "../locales/de.json";
import en from "../locales/en.json";
import es from "../locales/es.json";
import type { SessionView, Task, UiEvent } from "./api";
import {
  defaultStart,
  formErrors,
  lastRunText,
  nextRunText,
  reloadsTasks,
  scheduleText,
  skipText,
  sortTasks,
  type TaskFormInput,
} from "./tasks";

let t: TFunction;
let tEs: TFunction;
let tDe: TFunction;
beforeAll(async () => {
  const make = (lng: string) =>
    i18next.createInstance().init({
      lng,
      fallbackLng: "en",
      resources: { en: { translation: en }, es: { translation: es }, de: { translation: de } },
      interpolation: { escapeValue: false },
    });
  t = await make("en");
  tEs = await make("es");
  tDe = await make("de");
});

const task = (over: Partial<Task> = {}): Task => ({ project_id: "default",
  id: "t1",
  name: "Review",
  bot_id: "b1",
  prompt: "Review the open pull requests.",
  every_days: 7,
  at: "09:00",
  starts_on: "2027-06-07",
  enabled: true,
  next_run_at: "2027-06-14T12:00:00Z",
  last_run_at: null,
  last_run_session: null,
  skipped_at: null,
  created_at: "2027-06-01T08:00:00Z",
  updated_at: "2027-06-01T08:00:00Z",
  ...over,
});

const session = (id: string): SessionView => ({ project_id: "default", id }) as SessionView;

describe("scheduleText", () => {
  it("says every day for one day and every N days otherwise", () => {
    expect(scheduleText(t, task({ every_days: 1 }), "en-GB")).toBe("Every day at 09:00");
    expect(scheduleText(t, task({ every_days: 7 }), "en-GB")).toBe("Every 7 days at 09:00");
    expect(scheduleText(t, task({ every_days: 7, at: "18:30" }), "en-GB")).toBe("Every 7 days at 18:30");
  });
  it("follows the language", () => {
    expect(scheduleText(tEs, task({ every_days: 1 }), "es")).toBe("Cada día a las 9:00");
    expect(scheduleText(tEs, task({ every_days: 3 }), "es")).toBe("Cada 3 días a las 9:00");
  });
  it("writes the time in the locale's clock, as the next run is", () => {
    expect(scheduleText(t, task(), "en-US")).toMatch(/^Every 7 days at 9:00\sAM$/);
    expect(scheduleText(t, task({ at: "18:30" }), "en-US")).toMatch(/^Every 7 days at 6:30\sPM$/);
    expect(scheduleText(tDe, task(), "de")).toBe("Alle 7 Tage um 09:00");
    expect(scheduleText(tDe, task({ at: "18:30" }), "de")).toBe("Alle 7 Tage um 18:30");
  });
});

describe("nextRunText", () => {
  it("formats the next run in the locale", () => {
    expect(nextRunText(t, task(), "en")).toMatch(/^Jun 14, 2027, \d{1,2}:\d{2}/);
    expect(nextRunText(t, task(), "de")).toMatch(/^14\.06\.2027, \d{2}:\d{2}$/);
  });
  it("says off for a task that is off or has no next run", () => {
    expect(nextRunText(t, task({ enabled: false }), "en")).toBe("Off");
    expect(nextRunText(t, task({ next_run_at: null }), "en")).toBe("Off");
  });
});

describe("lastRunText", () => {
  const sessions = [session("s1")];
  it("says never before a first run", () => {
    expect(lastRunText(t, task(), sessions, "en")).toEqual({ text: "never", sessionId: null });
  });
  it("says removed when the session is gone", () => {
    const gone = task({ last_run_at: "2027-06-07T12:00:00Z", last_run_session: null });
    expect(lastRunText(t, gone, sessions, "en")).toEqual({ text: "removed", sessionId: null });
    const unknown = task({ last_run_at: "2027-06-07T12:00:00Z", last_run_session: "s9" });
    expect(lastRunText(t, unknown, sessions, "en")).toEqual({ text: "removed", sessionId: null });
  });
  it("gives the date and the session to open", () => {
    const ran = task({ last_run_at: "2027-06-07T12:00:00Z", last_run_session: "s1" });
    const last = lastRunText(t, ran, sessions, "en");
    expect(last.sessionId).toBe("s1");
    expect(last.text).toMatch(/^Jun 7, 2027, \d{1,2}:\d{2}/);
  });
});

describe("skipText", () => {
  it("shows a skip that came after the last run", () => {
    const text = skipText(t, task({ skipped_at: "2027-06-14T12:00:00Z", last_run_at: "2027-06-07T12:00:00Z" }), "en");
    expect(text).toMatch(/^Skipped Jun 1[34], .+: the last run was still busy$/);
  });
  it("shows a skip when there was no run", () => {
    expect(skipText(t, task({ skipped_at: "2027-06-14T12:00:00Z" }), "en")).not.toBeNull();
  });
  it("shows nothing when there is no skip or a run came after it", () => {
    expect(skipText(t, task(), "en")).toBeNull();
    const after = task({ skipped_at: "2027-06-07T12:00:00Z", last_run_at: "2027-06-14T12:00:00Z" });
    expect(skipText(t, after, "en")).toBeNull();
  });
});

describe("sortTasks", () => {
  it("sorts by next run, tasks that are off last, then by name", () => {
    const a = task({ id: "a", name: "Alpha", next_run_at: "2027-06-20T12:00:00Z" });
    const b = task({ id: "b", name: "Beta", next_run_at: "2027-06-10T12:00:00Z" });
    const c = task({ id: "c", name: "Charlie", enabled: false, next_run_at: null });
    const d = task({ id: "d", name: "Delta", enabled: false, next_run_at: null });
    const e = task({ id: "e", name: "Able", next_run_at: "2027-06-10T12:00:00Z" });
    expect(sortTasks([d, a, c, b, e]).map((x) => x.id)).toEqual(["e", "b", "a", "c", "d"]);
  });
  it("does not change its argument", () => {
    const list = [task({ id: "x", enabled: false, next_run_at: null }), task({ id: "y" })];
    sortTasks(list);
    expect(list.map((x) => x.id)).toEqual(["x", "y"]);
  });
});

describe("formErrors", () => {
  const ok: TaskFormInput = {
    name: "Review",
    bot_id: "b1",
    prompt: "Do it",
    every_days: 7,
    at: "09:00",
    starts_on: "2027-06-14",
  };
  it("accepts a good input", () => {
    expect(formErrors(ok)).toEqual({});
  });
  it("needs a name of 1 to 40 characters after trimming", () => {
    expect(formErrors({ ...ok, name: "   " }).name).toBe("taskForm.nameRequired");
    expect(formErrors({ ...ok, name: "x".repeat(40) }).name).toBeUndefined();
    expect(formErrors({ ...ok, name: "x".repeat(41) }).name).toBe("taskForm.nameTooLong");
    expect(formErrors({ ...ok, name: `  ${"x".repeat(40)}  ` }).name).toBeUndefined();
  });
  it("needs a bot and a prompt that is not blank", () => {
    expect(formErrors({ ...ok, bot_id: "" }).bot_id).toBe("taskForm.botRequired");
    expect(formErrors({ ...ok, prompt: " \n " }).prompt).toBe("taskForm.promptRequired");
  });
  it("needs every to be a whole number of 1 to 365 days", () => {
    for (const bad of [0, 366, 1.5, Number.NaN, -3]) expect(formErrors({ ...ok, every_days: bad }).every_days).toBe("taskForm.everyRange");
    for (const good of [1, 365]) expect(formErrors({ ...ok, every_days: good }).every_days).toBeUndefined();
  });
  it("needs a time as HH:MM", () => {
    for (const bad of ["", "9:00", "24:00", "09:60", "ab:cd"]) expect(formErrors({ ...ok, at: bad }).at).toBe("taskForm.timeInvalid");
    for (const good of ["00:00", "09:05", "23:59"]) expect(formErrors({ ...ok, at: good }).at).toBeUndefined();
  });
  it("needs a real date", () => {
    for (const bad of ["", "2027-02-30", "2027-13-01", "14/06/2027", "2027-6-1"]) {
      expect(formErrors({ ...ok, starts_on: bad }).starts_on).toBe("taskForm.startInvalid");
    }
    expect(formErrors({ ...ok, starts_on: "2028-02-29" }).starts_on).toBeUndefined();
  });
});

describe("reloadsTasks", () => {
  it("reacts to tasks_changed only", () => {
    expect(reloadsTasks({ type: "tasks_changed" })).toBe(true);
    expect(reloadsTasks({ type: "skills_changed" })).toBe(false);
    expect(reloadsTasks({ type: "paused", paused: true } as UiEvent)).toBe(false);
  });
});

describe("defaultStart", () => {
  it("is the local date of the next run for an edit", () => {
    const next = new Date(2027, 5, 14, 9, 0);
    expect(defaultStart(task({ next_run_at: next.toISOString() }), new Date(2027, 5, 10), "09:00")).toBe("2027-06-14");
  });
  it("keeps the start date of a task that is off", () => {
    expect(defaultStart(task({ enabled: false, next_run_at: null, starts_on: "2027-06-07" }), new Date(2027, 5, 10), "09:00")).toBe(
      "2027-06-07",
    );
  });
  it("is today for a new task whose time is still ahead", () => {
    expect(defaultStart(null, new Date(2027, 0, 5, 8, 30), "09:00")).toBe("2027-01-05");
    expect(defaultStart(null, new Date(2027, 0, 5, 8, 59, 59), "09:00")).toBe("2027-01-05");
  });
  it("is tomorrow for a new task whose time has come or passed", () => {
    expect(defaultStart(null, new Date(2027, 0, 5, 9, 0), "09:00")).toBe("2027-01-06");
    expect(defaultStart(null, new Date(2027, 0, 5, 23, 30), "09:00")).toBe("2027-01-06");
    expect(defaultStart(null, new Date(2027, 0, 31, 10, 0), "09:00")).toBe("2027-02-01");
  });
  it("is today for a new task while its time is not HH:MM", () => {
    expect(defaultStart(null, new Date(2027, 0, 5, 23, 30), "")).toBe("2027-01-05");
  });
});
