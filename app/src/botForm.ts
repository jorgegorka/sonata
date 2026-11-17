import type { Bot, Provider } from "./api";
import { isCommandError } from "./errors";
import { sameIds } from "./skills";

export const PROVIDERS: Provider[] = ["claude", "codex", "grok", "opencode"];

export function modelPlaceholderKey(provider: Provider): string {
  return provider === "opencode" ? "createBot.modelPlaceholderOpencode" : "createBot.modelPlaceholder";
}

export interface BotDraft {
  display_name: string;
  color: string;
  provider: Provider;
  model: string;
  instructions: string;
  skill_ids: string[];
}

export function draftOf(bot: Bot): BotDraft {
  return {
    display_name: bot.display_name,
    color: bot.color,
    provider: bot.provider,
    model: bot.model ?? "",
    instructions: bot.instructions,
    skill_ids: bot.skill_ids,
  };
}

export function editNotes(bot: Bot, draft: BotDraft): string[] {
  const reachesCli =
    draft.display_name.trim() !== bot.display_name ||
    draft.provider !== bot.provider ||
    (draft.model.trim() || null) !== bot.model ||
    draft.instructions !== bot.instructions ||
    !sameIds(draft.skill_ids, bot.skill_ids);
  if (!reachesCli) return [];
  if (draft.provider !== bot.provider) return ["botForm.appliesNext", "botForm.providerSwitch"];
  if (bot.provider === "grok") return ["botForm.appliesNext", "botForm.grokNewConversation"];
  if (bot.provider === "opencode" && (draft.model.trim() || null) !== (bot.model?.trim() || null)) {
    return ["botForm.appliesNext", "botForm.opencodeNewModel"];
  }
  return ["botForm.appliesNext"];
}

export function deletedAnyway(e: unknown): boolean {
  return isCommandError(e) && (e.code === "trashFailed" || e.code === "skillTrashFailed");
}
