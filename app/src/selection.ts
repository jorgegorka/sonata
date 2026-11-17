import type { SessionView } from "./api";

export type Selection =
  | { kind: "session"; id: string }
  | { kind: "newSession"; botId: string }
  | { kind: "thread"; id: string }
  | { kind: "newRun"; workflowId: string };

export function isChat(page: Selection | { kind: string } | null): page is Selection {
  return page?.kind === "session" || page?.kind === "newSession" || page?.kind === "thread";
}

export function followsFirstSend(page: Selection | { kind: string } | null, session: SessionView): boolean {
  return page?.kind === "newSession" && "botId" in page && page.botId === session.bot_id;
}
