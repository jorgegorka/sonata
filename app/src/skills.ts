import type { UiEvent } from "./api";

export interface SkillDraft {
  name: string;
  description: string;
  body: string;
}

export const EMPTY_SKILL: SkillDraft = { name: "", description: "", body: "" };

export function toggleId(ids: string[], id: string): string[] {
  const set = ids.filter((x, i) => ids.indexOf(x) === i);
  return set.includes(id) ? set.filter((x) => x !== id) : [...set, id];
}

export function sameIds(a: string[], b: string[]): boolean {
  const sa = [...new Set(a)].sort();
  const sb = [...new Set(b)].sort();
  return sa.length === sb.length && sa.every((x, i) => x === sb[i]);
}

export function reloadsSkills(event: UiEvent): boolean {
  return event.type === "skills_changed" || event.type === "bot_added" || event.type === "bot_updated";
}

export function canSave(draft: SkillDraft): boolean {
  return draft.name.trim().length > 0 && draft.description.trim().length > 0;
}

export function usedByKey(usedBy: string[]): "skills.usedBy" | "skills.unused" {
  return usedBy.length > 0 ? "skills.usedBy" : "skills.unused";
}
