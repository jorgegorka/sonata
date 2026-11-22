import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { BotView, Message, SessionView, ThreadSummary } from "../api";
import { threadName } from "../state";
import MessageItem from "./MessageItem";
import PanelGlyph from "./PanelGlyph";

interface Props {
  thread: ThreadSummary;
  bots: BotView[];
  sessions: SessionView[];
  messages: Message[];
  onError: (error: unknown) => void;
  browserOpen: boolean;
  onToggleBrowser: () => void;
  onBackToArchive?: () => void;
  onBack?: () => void;
  readOnly?: boolean;
}

export default function ThreadView({
  thread,
  bots,
  sessions,
  messages,
  onError,
  browserOpen,
  onToggleBrowser,
  onBackToArchive,
  onBack,
  readOnly = onBackToArchive !== undefined,
}: Props) {
  const { t } = useTranslation();
  const browserLabel = browserOpen ? t("browser.hide") : t("browser.show");
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length]);

  const nameOf = (id: string) => bots.find((b) => b.id === id)?.display_name ?? t("thread.deletedBot");
  return (
    <section className="chat">
      <header className="chat-header">
        <div>
          <div className="chat-title">{threadName(thread, sessions, bots, t)}</div>
          <div className="muted">{t(readOnly ? "thread.archived" : "thread.readOnly")}</div>
        </div>
        <div className="header-actions">
          {onBack && (
            <button className="link-button" onClick={onBack}>
              {t("board.back")}
            </button>
          )}
          {onBackToArchive && (
            <button className="link-button" onClick={onBackToArchive}>
              ← {t("archive.title")}
            </button>
          )}
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
            mine={false}
            senderName={nameOf(m.sender)}
            onError={onError}
            readOnly={readOnly}
          />
        ))}
        <div ref={endRef} />
      </div>
    </section>
  );
}
