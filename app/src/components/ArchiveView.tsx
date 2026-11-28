import { PageHeader } from "./ui";
import { useTranslation } from "react-i18next";
import type { BotView, SessionView, ThreadSummary } from "../api";
import { archivedDate, archivedGroups, archivedThreads } from "../archive";
import { avatarTextColor } from "../colors";
import type { Selection } from "../selection";
import { sessionLabel, threadName } from "../state";

interface Props {
  bots: BotView[];
  sessions: SessionView[];
  threads: ThreadSummary[];
  onOpen: (selection: Selection) => void;
}

export default function ArchiveView({ bots, sessions, threads, onOpen }: Props) {
  const { t, i18n } = useTranslation();
  const groups = archivedGroups(bots, sessions);
  const archived = archivedThreads(threads, sessions);
  return (
    <div className="page resource-page">
      <PageHeader title={t("archive.title")} description={t("archive.intro")} />
      {groups.length === 0 && archived.length === 0 && <p className="empty-state">{t("archive.empty")}</p>}
      {groups.map(({ bot, sessions: own }) => (
        <section key={bot.id} className="archive-group">
          <h2 className="archive-bot">
            <span className="avatar" style={{ background: bot.color, color: avatarTextColor(bot.color) }}>
              {bot.display_name.slice(0, 1).toUpperCase()}
            </span>
            {bot.display_name}
          </h2>
          <ul className="resource-list">
            {own.map((s) => (
              <li key={s.id}>
                <button className="resource-row" onClick={() => onOpen({ kind: "session", id: s.id })}>
                  <strong>{sessionLabel(s, sessions, bots, t)}</strong>
                  <span className="muted">{t("archive.archivedOn", { date: archivedDate(s, i18n.language) })}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {archived.length > 0 && (
        <section className="archive-group">
          <h2>{t("archive.threads")}</h2>
          <ul className="resource-list">
            {archived.map((th) => (
              <li key={th.id}>
                <button className="resource-row" onClick={() => onOpen({ kind: "thread", id: th.id })}>
                  <strong>{threadName(th, sessions, bots, t)}</strong>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
