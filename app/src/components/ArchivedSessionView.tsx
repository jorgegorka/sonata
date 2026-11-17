import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, USER, type BotView, type Message, type SessionView } from "../api";
import { archivedDate } from "../archive";
import { avatarTextColor } from "../colors";
import { confirmRemoval } from "../removal";
import MessageItem, { senderName } from "./MessageItem";
import PanelGlyph from "./PanelGlyph";
import SessionName from "./SessionName";

interface Props {
  bot: BotView;
  session: SessionView;
  label: string;
  cascade: number;
  messages: Message[];
  onRemoved: (ids: string[]) => void;
  onBack: () => void;
  onError: (error: unknown) => void;
  browserOpen: boolean;
  onToggleBrowser: () => void;
}

export default function ArchivedSessionView({
  bot,
  session,
  label,
  cascade,
  messages,
  onRemoved,
  onBack,
  onError,
  browserOpen,
  onToggleBrowser,
}: Props) {
  const { t, i18n } = useTranslation();
  const browserLabel = browserOpen ? t("browser.hide") : t("browser.show");
  const endRef = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState(false);

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

  async function remove() {
    const confirmed = await confirmRemoval(t, label, cascade).catch((e: unknown) => {
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
          <div className="chat-title" title={`${bot.display_name} · ${label}`}>
            {bot.display_name}{" "}
            <span className="chat-session">
              · <SessionName key={session.id} session={session} label={label} onError={onError} />
            </span>
          </div>
          <div className="muted">
            {t("archive.archivedOn", { date: archivedDate(session, i18n.language) })} · {t("archive.readOnly")}
          </div>
        </div>
        <div className="header-actions">
          <button className="link-button" onClick={onBack}>
            ← {t("archive.title")}
          </button>
          <button
            className="primary"
            disabled={busy}
            onClick={() => void act(async () => void (await api.restoreSession(session.id)))}
          >
            {t("archive.restore")}
          </button>
          <button className="danger" disabled={busy} onClick={() => void remove()}>
            {t("chat.removeSession")}
          </button>
          <button onClick={() => void api.openBotFolder(bot.id).catch(onError)}>{t("chat.showFolder")}</button>
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
        {messages.length === 0 && <p className="muted empty">{t("thread.empty")}</p>}
        {messages.map((m) => (
          <MessageItem
            key={m.id}
            message={m}
            mine={m.sender === USER}
            senderName={senderName(m, bot.display_name, t)}
            onError={onError}
            readOnly
          />
        ))}
        <div ref={endRef} />
      </div>
    </section>
  );
}
