export interface Draft {
  text: string;
  files: string[];
}

export const EMPTY_DRAFT: Draft = { text: "", files: [] };

export function withDraft(drafts: Record<string, Draft>, key: string, draft: Draft): Record<string, Draft> {
  const next = { ...drafts };
  if (draft.text === "" && draft.files.length === 0) delete next[key];
  else next[key] = draft;
  return next;
}
