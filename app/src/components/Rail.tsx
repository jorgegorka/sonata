import { Icon } from "./ui";
import { useTranslation } from "react-i18next";
import type { BotView, SessionView } from "../api";
import { avatarTextColor } from "../colors";
import { botWaits } from "../state";

interface Props {
  bots: BotView[];
  sessions: SessionView[];
  boardOpen: boolean;
  workflowsOpen: boolean;
  tasksOpen: boolean;
  archiveOpen: boolean;
  settingsOpen: boolean;
  onBoard: () => void;
  onBot: (botId: string) => void;
  onNewBot: () => void;
  onWorkflows: () => void;
  onTasks: () => void;
  onArchive: () => void;
  onSettings: () => void;
}

export default function Rail({
  bots,
  sessions,
  boardOpen,
  workflowsOpen,
  tasksOpen,
  archiveOpen,
  settingsOpen,
  onBoard,
  onBot,
  onNewBot,
  onWorkflows,
  onTasks,
  onArchive,
  onSettings,
}: Props) {
  const { t } = useTranslation();
  const rowClass = (selected: boolean) => (selected ? "bot-row selected" : "bot-row");
  const tip = (text: string) => ({ "aria-label": text, title: text });
  const waitingLabel = t("sidebar.waitingForYou");
  return (
    <nav className="sidebar collapsed">
      <button className={rowClass(boardOpen) + " rail-board"} onClick={onBoard} {...tip(t("sidebar.board"))}>
        <Icon name="board" />
      </button>
      <ul>
        {bots.map((b) => {
          const status = t(`status.${b.status}`);
          const waits = botWaits(sessions, b.id);
          const label = `${b.display_name} · ${status}${waits ? ` · ${waitingLabel}` : ""}`;
          return (
            <li key={b.id}>
              <button className="bot-row" onClick={() => onBot(b.id)} {...tip(label)}>
                <span className="avatar" style={{ background: b.color, color: avatarTextColor(b.color) }}>
                  {b.display_name.slice(0, 1).toUpperCase()}
                </span>
                {waits && <span className="waiting-mark" role="img" aria-label={waitingLabel} title={waitingLabel} />}
                <span className={`status-dot ${b.status}`} title={status} />
              </button>
            </li>
          );
        })}
      </ul>
      <button className="icon-button rail-new" onClick={onNewBot} {...tip(t("sidebar.newBot"))}>
        <Icon name="plus" />
      </button>
      <div className="sidebar-footer">
        <button className={rowClass(workflowsOpen)} onClick={onWorkflows} {...tip(t("sidebar.workflows"))}>
          <Icon name="workflow" />
        </button>
        <button className={rowClass(tasksOpen)} onClick={onTasks} {...tip(t("sidebar.tasks"))}>
          <Icon name="schedule" />
        </button>
        <button className={rowClass(archiveOpen)} onClick={onArchive} {...tip(t("sidebar.archive"))}>
          <Icon name="archive" />
        </button>
        <button className={rowClass(settingsOpen)} onClick={onSettings} {...tip(t("sidebar.settings"))}>
          <Icon name="settings" />
        </button>
      </div>
    </nav>
  );
}
