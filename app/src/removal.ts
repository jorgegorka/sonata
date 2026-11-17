import { ask } from "@tauri-apps/plugin-dialog";

type Translate = (key: string, options?: Record<string, string | number>) => string;

export function confirmRemoval(t: Translate, name: string, cascade: number): Promise<boolean> {
  const question = t("chat.removeSessionConfirm", { name });
  return ask(cascade > 0 ? `${question}\n\n${t("chat.removeSessionCascade", { count: cascade })}` : question, {
    title: t("chat.removeSession"),
    kind: "warning",
    okLabel: t("chat.removeSession"),
    cancelLabel: t("createBot.cancel"),
  });
}
