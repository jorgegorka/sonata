import i18next from "i18next";
import { describe, expect, it } from "vitest";
import de from "../locales/de.json";
import en from "../locales/en.json";
import es from "../locales/es.json";
import fr from "../locales/fr.json";

const SYSTEM_CODES = [
  "provider_unsupported",
  "session_reset",
  "config_new_session",
  "bot_start_failed",
  "bot_folder_outside",
  "prompt_failed",
  "bot_blocked",
  "no_output_captured",
  "chain_limit_reached",
  "bot_message_interrupted",
  "engine_error",
  "skill_folder_missing",
  "session_waiting",
  "usage_limit_wait",
  "usage_limit_retry",
  "master_added_skill",
  "master_created_bot",
  "master_created_task",
  "master_changed_task",
  "master_deleted_task",
  "master_created_workflow",
  "master_changed_workflow",
  "master_deleted_workflow",
  "master_archived_sessions",
  "master_changed_skill",
  "master_changed_unused_skill",
  "master_changed_bot",
  "workflow_finished",
];
const ERROR_CODES = [
  "internal", "trashFailed", "skillTrashFailed", "fileGone", "opensPrograms", "openFailed", "browserBadUrl",
  "browserFailed",
  "nameRequired", "providerUnavailable", "modelFormat", "messageEmpty", "fileNotFound", "fileIsFolder", "fileTooLarge",
  "fileCopyFailed", "botNotFound", "sessionNotFound", "sessionStarting", "sessionArchived", "messageNotFound", "attachmentNotFound",
  "alreadyDelivering", "notQueued",
  "notInterrupted", "notWorking", "stopFailed", "notWaiting", "notRunning", "answerFailed", "snapshotFailed",
  "botStarting", "databaseTooNew", "hopLimitRange", "botsFolderLocked", "badFolder", "botFolderOutside",
  "skillNameRequired", "skillDescriptionRequired", "skillNotFound", "skillInUse", "skillNoFile",
  "skillBadFrontmatter", "skillTooLarge", "languageUnknown",
  "taskNameRequired", "taskNameTooLong", "taskNameTaken", "taskPromptRequired", "taskEveryRange", "taskTimeInvalid",
  "taskStartInvalid", "taskNotFound",
  "workflowNameRequired", "workflowNameTooLong", "workflowNameTaken", "workflowWhenTooLong", "workflowNoSteps",
  "workflowTooManySteps", "workflowStepNameRequired", "workflowStepNameTooLong", "workflowStepNameReserved",
  "workflowStepNameTaken", "workflowStepNoBot", "workflowStepInstructionsRequired", "workflowNoStart",
  "workflowArrowTarget", "workflowOutcomeRequired", "workflowOutcomeTooLong", "workflowOutcomeLine",
  "workflowOutcomeTaken", "workflowTooManyArrows", "workflowUnreachable", "workflowBadDefinition",
  "workflowNotFound", "workflowRunNotFound", "workflowRunNotPaused", "workflowOutcomeUnknown",
  "masterMissing",
  "configFileTooLarge", "configFileNotYaml", "configFileWrongFormat", "configFileTooNew", "configFileTwoMasters", "configFileRepeats",
  "configExportTooLarge", "configBackupTooLarge", "skillSlugInvalid", "skillPathInvalid", "skillPathRepeated", "skillPathConflict", "skillFileContent",
  "skillNameNotSlug", "skillFolderTaken", "botColorInvalid", "providerUnknown", "skillUnknown", "importDependsOn",
];
const STATUSES = ["offline", "starting", "idle", "working", "needs_attention", "limited"];

describe("en.json", () => {
  it("has the labels of a picked element", () => {
    for (const key of ["page", "title", "selector", "text", "html"]) expect(en.message.element).toHaveProperty(key);
  });

  it("has a string for every engine system code", () => {
    for (const code of SYSTEM_CODES) expect(en.system).toHaveProperty(code);
  });

  it("has a label for every bot status", () => {
    for (const status of STATUSES) expect(en.status).toHaveProperty(status);
  });

  it("has a text for every error code", () => {
    for (const code of ERROR_CODES) expect(en.errors).toHaveProperty(code);
  });

  it("has the strings of the bot controls", () => {
    for (const key of ["continue", "cancel", "restart", "title", "dialog", "notRunning"]) {
      expect(en.attention).toHaveProperty(key);
    }
    for (const key of ["cancel", "cancelled", "retry", "retried", "failed"]) expect(en.message).toHaveProperty(key);
  });

  it("has the strings of the rail", () => {
    for (const key of ["board", "newBot", "waitingForYou", "tasks", "archive", "settings"]) {
      expect(en.sidebar).toHaveProperty(key);
    }
    expect(en.sidebar.board).toBe("Tasks");
    expect([en.sidebar.tasks, es.sidebar.tasks, fr.sidebar.tasks, de.sidebar.tasks]).toEqual([
      "Recurring tasks", "Tareas recurrentes", "Tâches récurrentes", "Wiederkehrende Aufgaben",
    ]);
    for (const key of [
      "bots", "showSessions", "hideSessions", "newSession", "collapse", "expand", "threads", "pause", "resume",
      "pausedNote",
    ]) {
      expect(en.sidebar).not.toHaveProperty(key);
    }
    for (const key of ["hideBrowser", "showBrowser"]) expect(en.menu).toHaveProperty(key);
    for (const key of ["hideSidebar", "showSidebar"]) expect(en.menu).not.toHaveProperty(key);
  });

  it("has the strings of the board", () => {
    for (const key of [
      "title", "running", "needsYou", "finished", "newTask", "pause", "resume", "pausedNote", "empty", "emptyColumn",
      "working", "starting", "queued", "paused", "asks", "blocked", "done", "cutOffTurn", "cutOffSent", "background",
      "owes", "moreNeeding", "memberNeedsYou", "answerPlaceholder", "send", "open", "markDone", "markDoneConfirm", "older",
      "markAllDone", "markAllDoneConfirm_one", "markAllDoneConfirm_other", "recurring", "unread", "more", "now",
      "minutes", "hours", "days", "back", "openFull", "close", "handoffs", "noHandoffs", "pickBot",
      "doneColumn", "showDone", "hideDone",
    ]) {
      expect(en).toHaveProperty(["board", key]);
    }
    expect(en.board.title).toBe("Tasks");
    expect(en.board.back).toBe("← Board");
    expect(en.board.openFull).toBe("Open the full chat");
    expect(en.board.handoffs).toBe("Hand-offs");
    expect(en.board.pickBot).toBe("Bot");
    expect(en.board.pause).toBe("Pause all bots");
    expect(en.board.resume).toBe("Resume all bots");
  });

  it("has the strings of a workflow run on the board, in the drawer and in the chat", () => {
    for (const key of [
      "pickBots", "pickWorkflows", "workflowPaused_visit_cap", "workflowPaused_no_result", "workflowPaused_bot_missing",
      "workflowAnotherRound", "workflowGoOn", "workflowRemind", "workflowStop",
    ]) {
      expect(en).toHaveProperty(["board", key]);
    }
    for (const key of ["workflowStep", "workflowReminder", "workflowResult", "workflowFinished"]) {
      expect(en.message).toHaveProperty(key);
    }
    expect(en).toHaveProperty(["workflowStart", "placeholder"]);
  });

  it("counts the older tasks Mark all done takes in the singular and the plural", async () => {
    const t = await i18next.createInstance().init({
      lng: "en",
      resources: { en: { translation: en }, de: { translation: de } },
      interpolation: { escapeValue: false },
    });
    expect(t("board.markAllDoneConfirm", { count: 1 })).toContain("1 older task done?");
    expect(t("board.markAllDoneConfirm", { count: 3 })).toContain("3 older tasks done?");
    expect(t("board.markAllDoneConfirm", { count: 2, lng: "de" })).toContain("2 ältere Aufgaben");
  });

  it("has the strings of a session's chat", () => {
    for (const key of [
      "newSession", "closeSession", "closeSessionConfirm", "removeSession", "removeSessionConfirm",
      "removeSessionCascade_one", "removeSessionCascade_other", "archiveSession", "archiveSessionHint", "rename",
    ]) {
      expect(en.chat).toHaveProperty(key);
    }
    for (const key of ["numbered", "openedBy"]) expect(en.sessions).toHaveProperty(key);
  });

  it("counts the sessions a removal takes in the singular and the plural", async () => {
    const t = await i18next.createInstance().init({
      lng: "en",
      resources: { en: { translation: en }, de: { translation: de } },
      interpolation: { escapeValue: false },
    });
    expect(t("chat.removeSessionCascade", { count: 1 })).toBe("1 session it opened in another bot goes with it.");
    expect(t("chat.removeSessionCascade", { count: 3 })).toBe("3 sessions it opened in other bots go with it.");
    expect(t("chat.removeSessionCascade", { count: 2, lng: "de" })).toContain("2 Sitzungen");
  });

  it("has the strings of the archive", () => {
    for (const key of ["title", "intro", "empty", "archivedOn", "readOnly", "restore", "threads"]) {
      expect(en.archive).toHaveProperty(key);
    }
    expect(en.sidebar).toHaveProperty("archive");
    expect(en.thread).toHaveProperty("archived");
  });

  it("has the strings of the browser panel", () => {
    for (const key of ["title", "back", "forward", "reload", "address", "openInBrowser", "close", "stalled", "hide", "show"]) {
      expect(en.browser).toHaveProperty(key);
    }
  });

  it("has the strings of the recurring tasks", () => {
    for (const key of [
      "title", "intro", "new", "empty", "every_one", "every_other", "next", "off", "last", "never", "removed",
      "skipped", "on", "switch", "runNow", "running", "edit", "delete", "deleteTitle", "deleteConfirm",
    ]) {
      expect(en.tasks).toHaveProperty(key);
    }
    for (const key of [
      "createTitle", "editTitle", "name", "namePlaceholder", "bot", "prompt", "promptHint", "every", "days", "at",
      "startingOn", "create", "creating", "save", "saving", "cancel", "noBots", "nameRequired", "nameTooLong",
      "botRequired", "promptRequired", "everyRange", "timeInvalid", "startInvalid",
    ]) {
      expect(en.taskForm).toHaveProperty(key);
    }
    expect(en.sidebar).toHaveProperty("tasks");
    expect(en.message).toHaveProperty("taskRun");
    for (const key of ["deleteConfirmTasks_one", "deleteConfirmTasks_other"]) expect(en.botForm).toHaveProperty(key);
  });

  it("has the strings of picking an element", () => {
    for (const key of ["pick", "stopPick", "pickUnavailable", "pickNoChat", "pickRefused"]) expect(en.browser).toHaveProperty(key);
    for (const key of ["to", "hint", "placeholder", "keys", "send", "sending", "failed", "noTarget", "tooLong"]) {
      expect(en).toHaveProperty(["pick", key]);
    }
    expect(en.pick).not.toHaveProperty("changed");
  });
});

describe("the project folder hints", () => {
  for (const [code, file] of [["en", en], ["es", es], ["fr", fr], ["de", de]] as const) {
    it(`${code}.json counts Master among the Sonata bots`, () => {
      expect(file.setup.projectHint).toContain("Master");
      expect(file.settings.projectHint).toContain("Master");
    });
  }
  it("describe the first project's directory and default provider", () => {
    expect(en.setup.projectHint).toContain("source directory and default AI provider");
    expect(en.settings.projectHint).toContain("Master and bots it creates");
    expect(en.settings.projectHint).not.toContain("eight");
  });
});

function keyPaths(value: unknown, prefix = ""): string[] {
  if (typeof value !== "object" || value === null) return [prefix];
  return Object.entries(value)
    .flatMap(([k, v]) => keyPaths(v, prefix ? `${prefix}.${k}` : k))
    .sort();
}

function valueAt(value: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((v, part) => (v as Record<string, unknown>)[part], value);
}

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort();
}

describe("es, fr and de", () => {
  const paths = keyPaths(en);
  for (const [code, file] of [["es", es], ["fr", fr], ["de", de]] as const) {
    it(`${code}.json has the keys of en.json, no empty value, and the same placeholders`, () => {
      expect(keyPaths(file)).toEqual(paths);
      for (const path of paths) {
        const text = valueAt(file, path);
        expect(typeof text === "string" && text.trim().length > 0, `${code}: ${path} is empty`).toBe(true);
        expect(placeholders(text as string), `${code}: ${path} placeholders`).toEqual(
          placeholders(valueAt(en, path) as string),
        );
      }
    });
  }
});
