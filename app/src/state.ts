import {
  USER,
  type Bot,
  type BotView,
  type Message,
  type SessionView,
  type SkillView,
  type ThreadSummary,
  type UiEvent,
  type UsageLimit,
} from "./api";

export interface AppState {
  bots: BotView[];
  sessions: SessionView[];
  removed: string[];
  threads: ThreadSummary[];
  messages: Record<string, Message[]>;
  paused: boolean;
  skills: SkillView[];
  usageLimits: UsageLimit[];
}

export const initialState: AppState = {
  bots: [],
  sessions: [],
  removed: [],
  threads: [],
  messages: {},
  paused: false,
  skills: [],
  usageLimits: [],
};

export type Action =
  | { type: "bots_loaded"; bots: BotView[] }
  | { type: "bot_created"; bot: Bot }
  | { type: "bot_updated"; bot: Bot }
  | { type: "bot_deleted"; botId: string }
  | { type: "sessions_loaded"; sessions: SessionView[] }
  | { type: "threads_loaded"; threads: ThreadSummary[] }
  | { type: "paused_loaded"; paused: boolean }
  | { type: "skills_loaded"; skills: SkillView[] }
  | { type: "limits_loaded"; limits: UsageLimit[] }
  | { type: "messages_loaded"; key: string; messages: Message[] }
  | { type: "ui_event"; event: UiEvent };

export function isBotThread(message: Message): boolean {
  return message.sender !== USER && message.recipient !== USER;
}

export function conversationOf(message: Message): string {
  if (message.sender === USER) return message.recipient_session ?? message.thread_id;
  if (message.recipient === USER) return message.sender_session ?? message.thread_id;
  return message.thread_id;
}

export function isArchived(session: SessionView): boolean {
  return session.archived_at !== null;
}

export function isWaiting(session: SessionView): boolean {
  return session.waiting_since !== null && !isArchived(session);
}

export function botWaits(sessions: SessionView[], botId: string): boolean {
  return sessions.some((s) => s.bot_id === botId && isWaiting(s));
}

export function showsArchived(session: SessionView | null, archiving: string | null): boolean {
  return session !== null && isArchived(session) && session.id !== archiving;
}

export function hasArchivedEnd(thread: ThreadSummary, sessions: SessionView[]): boolean {
  return sessions.some((s) => (s.id === thread.session_a || s.id === thread.session_b) && isArchived(s));
}

export function sessionsOf(sessions: SessionView[], botId: string): SessionView[] {
  return sessions
    .filter((s) => s.bot_id === botId && !isArchived(s))
    .sort((a, b) =>
      a.last_activity_at === b.last_activity_at
        ? b.number - a.number
        : a.last_activity_at < b.last_activity_at
          ? 1
          : -1,
    );
}

export type Translate = (key: string, options?: Record<string, string | number>) => string;

function ownTitle(session: SessionView): string | null {
  return session.title !== null && session.title.trim() !== "" ? session.title : null;
}

export function sessionLabel(session: SessionView, sessions: SessionView[], bots: BotView[], t: Translate): string {
  const title = ownTitle(session);
  if (title) return title;
  const asker = session.opened_by ? sessions.find((s) => s.id === session.opened_by) : undefined;
  const handle = asker ? handleOf(bots, asker.bot_id) : null;
  if (asker && handle) {
    return t("sessions.openedBy", { handle, label: ownTitle(asker) ?? t("sessions.numbered", { n: asker.number }) });
  }
  return t("sessions.numbered", { n: session.number });
}

export function threadName(thread: ThreadSummary, sessions: SessionView[], bots: BotView[], t: Translate): string {
  const end = (participant: string, sessionId: string | null, other: string | null) => {
    const handle = handleOf(bots, participant) ?? t("thread.deletedBot");
    const session = sessionId ? sessions.find((s) => s.id === sessionId) : undefined;
    if (!session || (!ownTitle(session) && session.opened_by !== null && session.opened_by === other)) return handle;
    return `${handle} · ${sessionLabel(session, sessions, bots, t)}`;
  };
  const a = end(thread.participant_a, thread.session_a, thread.session_b);
  const b = end(thread.participant_b, thread.session_b, thread.session_a);
  return `${a} ↔ ${b}`;
}

function openedBy(sessions: SessionView[], sessionId: string): Set<string> {
  const found = new Set<string>();
  const queue = [sessionId];
  for (let id = queue.pop(); id !== undefined; id = queue.pop()) {
    for (const s of sessions) {
      if (s.opened_by === id && s.id !== sessionId && !found.has(s.id)) {
        found.add(s.id);
        queue.push(s.id);
      }
    }
  }
  return found;
}

export function renamedTo(draft: string, title: string | null): string | null {
  const name = draft.trim();
  return name === (title ?? "").trim() ? null : name;
}

export function removalCount(sessions: SessionView[], sessionId: string): number {
  return openedBy(sessions, sessionId).size;
}

export function handleOf(bots: BotView[], id: string): string | null {
  const bot = bots.find((b) => b.id === id);
  return bot ? `@${bot.handle}` : null;
}

function upsert(list: Message[], message: Message): Message[] {
  const index = list.findIndex((m) => m.id === message.id);
  if (index === -1) return [...list, message];
  const next = list.slice();
  next[index] = message;
  return next;
}

function upsertSession(list: SessionView[], session: SessionView): SessionView[] {
  const index = list.findIndex((s) => s.id === session.id);
  if (index === -1) return [...list, session];
  const next = list.slice();
  next[index] = session;
  return next;
}

function withoutSessions(sessions: SessionView[], gone: string[]): SessionView[] {
  return sessions
    .filter((s) => !gone.includes(s.id))
    .map((s) => (s.opened_by !== null && gone.includes(s.opened_by) ? { ...s, opened_by: null } : s));
}

function withoutKeys(messages: Record<string, Message[]>, keys: string[]): Record<string, Message[]> {
  return Object.fromEntries(Object.entries(messages).filter(([key]) => !keys.includes(key)));
}

function touchThread(threads: ThreadSummary[], message: Message): ThreadSummary[] {
  const known = threads.find((t) => t.id === message.thread_id);
  const [[a, sa], [b, sb]] = (
    [
      [message.sender, message.sender_session ?? null],
      [message.recipient, message.recipient_session ?? null],
    ] as [string, string | null][]
  ).sort((x, y) => (x[0] < y[0] ? -1 : x[0] > y[0] ? 1 : 0));
  const thread: ThreadSummary = known
    ? { ...known, last_message_at: message.created_at }
    : {
        id: message.thread_id,
        participant_a: a,
        participant_b: b,
        session_a: sa,
        session_b: sb,
        last_message_at: message.created_at,
      };
  return [thread, ...threads.filter((t) => t.id !== message.thread_id)];
}

export function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case "bots_loaded":
      return { ...state, bots: action.bots };
    case "bot_created":
      if (state.bots.some((b) => b.id === action.bot.id)) return state;
      return { ...state, bots: [...state.bots, { ...action.bot, status: "offline" }] };
    case "bot_updated":
      return {
        ...state,
        bots: state.bots.map((b) => (b.id === action.bot.id ? { ...action.bot, status: b.status } : b)),
      };
    case "bot_deleted": {
      const gone = state.sessions.filter((s) => s.bot_id === action.botId).map((s) => s.id);
      return {
        ...state,
        bots: state.bots.filter((b) => b.id !== action.botId),
        sessions: withoutSessions(state.sessions, gone),
        removed: [...state.removed, ...gone],
        messages: withoutKeys(state.messages, gone),
      };
    }
    case "sessions_loaded": {
      const loaded = action.sessions.filter((s) => !state.removed.includes(s.id));
      const extra = state.sessions.filter((s) => !loaded.some((l) => l.id === s.id));
      return { ...state, sessions: [...loaded, ...extra] };
    }
    case "threads_loaded":
      return { ...state, threads: action.threads };
    case "paused_loaded":
      return { ...state, paused: action.paused };
    case "skills_loaded":
      return { ...state, skills: action.skills };
    case "limits_loaded":
      return { ...state, usageLimits: action.limits };
    case "messages_loaded": {
      if (state.removed.includes(action.key)) return state;
      const loaded = action.messages;
      const extra = (state.messages[action.key] ?? []).filter((m) => !loaded.some((l) => l.id === m.id));
      return { ...state, messages: { ...state.messages, [action.key]: [...loaded, ...extra] } };
    }
    case "ui_event": {
      const event = action.event;
      if (event.type === "bot_status") {
        return {
          ...state,
          bots: state.bots.map((b) => (b.id === event.bot_id ? { ...b, status: event.status } : b)),
        };
      }
      if (event.type === "paused") return { ...state, paused: event.paused };
      if (event.type === "usage_limits") return { ...state, usageLimits: event.limits };
      if (event.type === "bot_added") {
        if (state.bots.some((b) => b.id === event.bot.id)) return state;
        return { ...state, bots: [...state.bots, event.bot] };
      }
      if (event.type === "bot_updated") return reducer(state, { type: "bot_updated", bot: event.bot });
      if (event.type === "session_added" || event.type === "session_updated") {
        if (state.removed.includes(event.session.id)) return state;
        const known = state.sessions.find((s) => s.id === event.session.id);
        const session =
          event.type === "session_added" && known ? { ...event.session, status: known.status } : event.session;
        return { ...state, sessions: upsertSession(state.sessions, session) };
      }
      if (event.type === "session_status") {
        if (!state.sessions.some((s) => s.id === event.session_id)) return state;
        return {
          ...state,
          sessions: state.sessions.map((s) => (s.id === event.session_id ? { ...s, status: event.status } : s)),
        };
      }
      if (event.type === "session_removed") {
        const gone = [event.session_id];
        return {
          ...state,
          sessions: withoutSessions(state.sessions, gone),
          removed: [...state.removed, event.session_id, ...event.thread_ids],
          threads: state.threads.filter((t) => !event.thread_ids.includes(t.id)),
          messages: withoutKeys(state.messages, [event.session_id, ...event.thread_ids]),
        };
      }
      if (event.type !== "message_added" && event.type !== "message_updated") return state;
      const message = event.message;
      const key = conversationOf(message);
      if (state.removed.includes(key) || state.removed.includes(message.thread_id)) return state;
      const threads =
        event.type === "message_added" && isBotThread(message) ? touchThread(state.threads, message) : state.threads;
      return { ...state, threads, messages: { ...state.messages, [key]: upsert(state.messages[key] ?? [], message) } };
    }
  }
}
