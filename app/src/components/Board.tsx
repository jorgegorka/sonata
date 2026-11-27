import type { ReactNode } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, type BotView, type Glance, type SessionActivity, type SessionView, type UsageLimit, type WorkItem } from "../api";
import { hasWorkGoingOn, type BoardTask } from "../board";
import type { Translate } from "../state";
import WorkBoard from "./WorkBoard";
import { useWorkItems } from "./useWorkItems";
import type { BoardData } from "./useBoardData";
import type { TaskCreation } from "./WorkItemForm";

export interface Props {
  bots: BotView[];
  sessions: SessionView[];
  paused: boolean;
  limits: UsageLimit[];
  onTogglePause: () => void;
  onOpen: (task: BoardTask) => void;
  creation: TaskCreation | null;
  onCreate: (parent?: WorkItem, executor?: TaskCreation["executor"]) => void;
  onCloseCreation: () => void;
  onError: (e: unknown) => void;
  drawer?: (tasks: BoardTask[], now: Date, data: BoardData, context?: (task: BoardTask) => ReactNode) => ReactNode;
  data?: BoardData;
  now?: Date;
}

export async function markDone(
  task: BoardTask,
  activity: Record<string, SessionActivity>,
  glances: Record<string, Glance>,
  name: string,
  t: Translate,
): Promise<void> {
  if (task.workItem) { await api.archiveWorkItem(task.workItem.id, true); return; }
  if (task.orchestration?.taskKey) {
    await api.archiveOrchestrationTask(task.orchestration.run.id, task.orchestration.taskKey, true);
    return;
  }
  if (task.orchestration?.run.status === "active") return;
  if (hasWorkGoingOn(task, activity, glances)) {
    const confirmed = await ask(t("board.markDoneConfirm", { name }), {
      title: t("board.markDone"),
      kind: "warning",
      okLabel: t("board.markDone"),
      cancelLabel: t("createBot.cancel"),
    });
    if (!confirmed) return;
  }
  await api.archiveSession(task.id);
}

export default function Board(props: Props) {
  const work = useWorkItems();
  return <WorkBoard {...props} items={work.items} loading={!work.loaded && work.error === null}
    loadError={work.error} onRetry={work.reload} onSaved={work.saved} />;
}
