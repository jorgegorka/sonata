export type Selection = { kind: "session"; id: string } | { kind: "thread"; id: string };

export function isChat(page: Selection | { kind: string } | null): page is Selection {
  return page?.kind === "session" || page?.kind === "thread";
}
