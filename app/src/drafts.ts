import type { Selection } from "./selection";

export interface Draft {
  text: string;
  files: string[];
}

export const EMPTY_DRAFT: Draft = { text: "", files: [] };

export function newSessionKey(botId: string): string {
  return `new:${botId}`;
}

export function draftKey(selection: Selection | null): string | null {
  if (selection?.kind === "session") return selection.id;
  if (selection?.kind === "newSession") return newSessionKey(selection.botId);
  return null;
}

export function withDraft(drafts: Record<string, Draft>, key: string, draft: Draft): Record<string, Draft> {
  const next = { ...drafts };
  if (draft.text === "" && draft.files.length === 0) delete next[key];
  else next[key] = draft;
  return next;
}

export function moveDraft(drafts: Record<string, Draft>, botId: string, sessionId: string): Record<string, Draft> {
  const from = newSessionKey(botId);
  if (!(from in drafts)) return drafts;
  const { [from]: draft, ...rest } = drafts;
  return { ...rest, [sessionId]: draft };
}
