import { useProjectApi } from "../projectContext";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { USER, type BotView, type Message, type SessionView } from "../api";
import { avatarTextColor } from "../colors";
import type { Draft } from "../drafts";
import { confirmRemoval } from "../removal";
import AttentionCard from "./AttentionCard";
import Composer from "./Composer";
import MessageItem, { senderName } from "./MessageItem";
import PanelGlyph from "./PanelGlyph";
import SessionName from "./SessionName";
import TerminalView from "./TerminalView";

interface Props {
  bot: BotView;
  session: SessionView | null;
  label: string | null;
  cascade: number;
  onSent: (session: SessionView) => void;
  onRemoved: (ids: string[]) => void;
  onArchive: () => Promise<void>;
  focusComposer: boolean;
  messages: Message[];
  onError: (error: unknown) => void;
  onEdit: () => void;
  draft: Draft;
  onDraftChange: (update: (draft: Draft) => Draft) => void;
  browserOpen: boolean;
  onToggleBrowser: () => void;
  onBack?: () => void;
}

export default function Chat({
  bot,
  session,
  label,
  cascade,
  onSent,
  onRemoved,
  onArchive,
  focusComposer,
  messages,
  onError,
  onEdit,
  draft,
  onDraftChange,
  browserOpen,
  onToggleBrowser,
  onBack,
}: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const browserLabel = browserOpen ? t("browser.hide") : t("browser.show");
  const endRef = useRef<HTMLDivElement>(null);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const status = session?.status ?? "offline";
  const panelOpen = session !== null && terminalOpen && status !== "needs_attention";

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length]);

  async function act(action: () => Promise<void>) {
    setBusy(true);
    try {
      await action();
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  }

  async function close() {
    if (!session) return;
    const confirmed = await ask(t("chat.closeSessionConfirm", { name: label ?? t("chat.newSession") }), {
      title: t("chat.closeSession"),
      kind: "warning",
      okLabel: t("chat.closeSession"),
      cancelLabel: t("createBot.cancel"),
    }).catch((e: unknown) => {
      onError(e);
      return false;
    });
    if (confirmed) await act(() => api.stopSession(session.id));
  }

  async function remove() {
    if (!session) return;
    const confirmed = await confirmRemoval(t, label ?? t("chat.newSession"), cascade).catch((e: unknown) => {
      onError(e);
      return false;
    });
    if (confirmed) await act(async () => onRemoved(await api.removeSession(session.id)));
  }

  return (
    <section className="chat">
      <header className="chat-header">
        <span className="avatar" style={{ background: bot.color, color: avatarTextColor(bot.color) }}>
          {bot.display_name.slice(0, 1).toUpperCase()}
        </span>
        <div className="chat-heading">
          <div className="chat-title" title={`${bot.display_name} · ${label ?? t("chat.newSession")}`}>
            {bot.display_name}{" "}
            <span className="chat-session">
              ·{" "}
              {session && label ? (
                <SessionName key={session.id} session={session} label={label} onError={onError} />
              ) : (
                (label ?? t("chat.newSession"))
              )}
            </span>
          </div>
          <div className="muted">
            @{bot.handle} · {t(`providers.${bot.provider}`)}
            {session && (
              <>
                {" "}
                · <span className={`status-text ${status}`}>{t(`status.${status}`)}</span>
              </>
            )}
          </div>
        </div>
        <div className="header-actions">
          {onBack && (
            <button className="link-button" onClick={onBack}>
              {t("board.back")}
            </button>
          )}
          {session && status === "working" && (
            <button onClick={() => void api.stopTurn(session.id).catch(onError)}>■ {t("chat.stop")}</button>
          )}
          {session && status !== "needs_attention" && (
            <button onClick={() => setTerminalOpen(!panelOpen)}>
              {panelOpen ? t("terminal.hide") : t("terminal.show")}
            </button>
          )}
          {session && status !== "offline" && (
            <button disabled={busy} onClick={() => void close()}>
              {t("chat.closeSession")}
            </button>
          )}
          {session && (
            <button
              disabled={busy}
              title={t("chat.archiveSessionHint")}
              onClick={() => void act(onArchive)}
            >
              {t("chat.archiveSession")}
            </button>
          )}
          {session && (
            <button className="danger" disabled={busy} onClick={() => void remove()}>
              {t("chat.removeSession")}
            </button>
          )}
          <button onClick={() => void api.openBotFolder(bot.id).catch(onError)}>{t("chat.showFolder")}</button>
          <button onClick={onEdit}>{t("chat.edit")}</button>
          <button
            className="icon-button panel-toggle"
            onClick={onToggleBrowser}
            aria-label={browserLabel}
            title={browserLabel}
          >
            <PanelGlyph side="right" />
          </button>
        </div>
      </header>
      <div className="messages">
        {messages.length === 0 && <p className="muted empty">{t("chat.empty", { name: bot.display_name })}</p>}
        {messages.map((m) => (
          <MessageItem
            key={m.id}
            message={m}
            mine={m.sender === USER}
            senderName={senderName(m, bot.display_name, t)}
            onError={onError}
          />
        ))}
        <div ref={endRef} />
      </div>
      {session && status === "needs_attention" && <AttentionCard session={session} onError={onError} />}
      {panelOpen && (
        <section className="terminal-panel">
          <TerminalView sessionId={session.id} />
          <div className="actions">
            <button disabled={busy} onClick={() => void act(() => api.restartSession(session.id))}>
              {t("attention.restart")}
            </button>
            <button onClick={() => setTerminalOpen(false)}>{t("terminal.close")}</button>
          </div>
        </section>
      )}
      <Composer
        bot={bot}
        sessionId={session?.id ?? null}
        onSent={onSent}
        autoFocus={focusComposer}
        draft={draft}
        onDraftChange={onDraftChange}
        onError={onError}
        options={session?.waiting_options ?? []}
      />
    </section>
  );
}
