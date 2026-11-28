import { Icon } from "./ui";
import { useProjectApi } from "../projectContext";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { answerBody } from "../answer";
import { type BotView, type SessionView } from "../api";
import { EMPTY_DRAFT, type Draft } from "../drafts";
import { addFiles, fileName } from "../files";

interface Props {
  bot: BotView;
  sessionId: string | null;
  onSent: (session: SessionView) => void;
  autoFocus: boolean;
  draft: Draft;
  onDraftChange: (update: (draft: Draft) => Draft) => void;
  onError: (error: unknown) => void;
  options: string[];
  send?: (body: string, files: string[]) => Promise<SessionView>;
  placeholder?: string;
}

export default function Composer({
  bot,
  sessionId,
  onSent,
  autoFocus,
  draft,
  onDraftChange,
  onError,
  options,
  send: sendVia,
  placeholder: placeholderText,
}: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [dragging, setDragging] = useState(false);
  const [sending, setSending] = useState(false);
  const inFlight = useRef(false);
  const change = useRef(onDraftChange);
  change.current = onDraftChange;

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const drag = event.payload;
        if (drag.type === "drop") {
          setDragging(false);
          change.current((d) => ({ ...d, files: addFiles(d.files, drag.paths) }));
        } else {
          setDragging(drag.type !== "leave");
        }
      })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch(onError);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [onError]);

  async function pickFiles() {
    try {
      const picked = await open({ multiple: true, directory: false });
      if (picked) onDraftChange((d) => ({ ...d, files: addFiles(d.files, Array.isArray(picked) ? picked : [picked]) }));
    } catch (e) {
      onError(e);
    }
  }

  async function send(option?: string) {
    if (option !== undefined && inFlight.current) return;
    const body = option === undefined ? draft.text.trim() : answerBody(option, draft.text);
    if (!body && draft.files.length === 0) return;
    const unsent = draft;
    inFlight.current = true;
    setSending(true);
    onDraftChange(() => EMPTY_DRAFT);
    try {
      if (sendVia) {
        onSent(await sendVia(body, unsent.files));
      } else {
        const sent = await api.sendMessage(bot.id, sessionId, body, unsent.files);
        if (sessionId === null) onSent(sent.session);
      }
    } catch (e) {
      onDraftChange((current) => (current.text === "" && current.files.length === 0 ? unsent : current));
      onError(e);
    } finally {
      inFlight.current = false;
      setSending(false);
    }
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  const placeholder = placeholderText ?? t("chat.placeholder", { name: bot.display_name });
  return (
    <footer className={dragging ? "composer dragging" : "composer"}>
      {options.length > 0 && (
        <div className="answer-options">
          {options.map((option) => (
            <button key={option} disabled={sending} onClick={() => void send(option)}>
              {draft.text.trim() ? t("chat.optionWithComment", { option }) : option}
            </button>
          ))}
        </div>
      )}
      {draft.files.length > 0 && (
        <ul className="pending-files">
          {draft.files.map((f) => (
            <li key={f} className="chip">
              <Icon name="attach" /><span className="chip-label">{fileName(f)}</span>
              <button
                className="icon-button"
                aria-label={t("chat.removeFile", { name: fileName(f) })}
                onClick={() => onDraftChange((d) => ({ ...d, files: d.files.filter((x) => x !== f) }))}
              >
                <Icon name="close" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <textarea
        value={draft.text}
        onChange={(e) => {
          const text = e.target.value;
          onDraftChange((d) => ({ ...d, text }));
        }}
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        aria-label={placeholder}
        autoFocus={autoFocus}
        rows={3}
      />
      <div className="composer-row">
        <span className="muted">{dragging ? t("chat.dropHere") : t("chat.hint")}</span>
        <div className="actions">
          <button className="icon-button ghost" onClick={() => void pickFiles()} aria-label={t("chat.attach")} title={t("chat.attach")}>
            <Icon name="attach" />
          </button>
          <button
            className="primary"
            onClick={() => void send()}
            disabled={!draft.text.trim() && draft.files.length === 0}
          >
            {t("chat.send")}
          </button>
        </div>
      </div>
    </footer>
  );
}
