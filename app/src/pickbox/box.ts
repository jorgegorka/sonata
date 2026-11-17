import type { PickBoxState } from "../api";

export type BoxKey = "send" | "newline" | "close" | "pass";

export function boxKey(e: { key: string; shiftKey: boolean; isComposing: boolean; keyCode: number }): BoxKey {
  if (e.isComposing || e.keyCode === 229) return "pass";
  if (e.key === "Enter") return e.shiftKey ? "newline" : "send";
  if (e.key === "Escape") return "close";
  return "pass";
}

export const MAX_COMMENT = 4000;

export function trimComment(text: string): string {
  return text.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
}

export interface BoxView {
  target: string;
  canSend: boolean;
  status: string | null;
  alert: boolean;
}

export function boxView(state: PickBoxState, text: string): BoxView {
  const strings = state.strings;
  if (!strings) return { target: "", canSend: false, status: null, alert: false };
  const comment = trimComment(text);
  const tooLong = [...comment].length > MAX_COMMENT;
  const canSend = state.target !== null && state.status !== "sending" && comment !== "" && !tooLong;
  const target = state.target ?? strings.noTarget;
  if (state.status === "sending") return { target, canSend, status: strings.sending, alert: false };
  if (tooLong) return { target, canSend, status: strings.tooLong, alert: true };
  if (state.status === "failed") return { target, canSend, status: strings.failed, alert: true };
  return { target, canSend, status: null, alert: false };
}
