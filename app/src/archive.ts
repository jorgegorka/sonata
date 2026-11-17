import type { BotView, SessionView, ThreadSummary } from "./api";
import { hasArchivedEnd, isArchived } from "./state";

export interface ArchivedGroup {
  bot: BotView;
  sessions: SessionView[];
}

export function archivedGroups(bots: BotView[], sessions: SessionView[]): ArchivedGroup[] {
  const latestFirst = (a: SessionView, b: SessionView) =>
    a.archived_at === b.archived_at ? b.number - a.number : (a.archived_at ?? "") < (b.archived_at ?? "") ? 1 : -1;
  return bots
    .map((bot) => ({ bot, sessions: sessions.filter((s) => s.bot_id === bot.id && isArchived(s)).sort(latestFirst) }))
    .filter((group) => group.sessions.length > 0);
}

export function archivedThreads(threads: ThreadSummary[], sessions: SessionView[]): ThreadSummary[] {
  return threads.filter((th) => hasArchivedEnd(th, sessions));
}

export function archivedDate(session: SessionView, language: string): string {
  return session.archived_at ? new Date(session.archived_at).toLocaleDateString(language, { dateStyle: "medium" }) : "";
}
