import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { TFunction } from "i18next";
import { memo, useContext, useRef, useState, type AnchorHTMLAttributes } from "react";
import { linkAction } from "../links";
import { OpenLinkContext } from "../openLink";
import { api, USER, type Message, type WorkflowMark } from "../api";
import { opensPrograms } from "../files";
import { systemParams } from "../errors";
import { formatReset } from "../limits";
import { elementSummary, splitPageElements } from "../pick";

export function MessageLink({ href, children }: AnchorHTMLAttributes<HTMLAnchorElement>) {
  const openInline = useContext(OpenLinkContext);
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        const action = linkAction(href, e);
        if (action === "ignore" || !href) return;
        if (action === "inline" && openInline) openInline(href);
        else void openUrl(href);
      }}
    >
      {children}
    </a>
  );
}

function resetTime(params: Record<string, string> | null, language: string): { time?: string } {
  return params?.resets_at ? { time: formatReset(params.resets_at, new Date(), language) } : {};
}

function markLabel(mark: WorkflowMark, t: TFunction): string {
  const { workflow, step, visit } = mark;
  switch (mark.role) {
    case "request":
      return t("message.workflowStep", { workflow, step, visit });
    case "reminder":
      return t("message.workflowReminder", { step });
    case "result":
      return t("message.workflowResult", { step }) + (mark.outcome ? ` → ${mark.outcome}` : "");
    case "finished":
      return t("message.workflowFinished", { workflow });
  }
}

export function senderName(message: Message, botName: string, t: TFunction): string {
  if (message.sender !== USER) return botName;
  return message.workflow_mark || message.system_code === "orchestration_event" ? t("app.title") : t("message.you");
}

interface Props {
  message: Message;
  senderName: string;
  mine: boolean;
  onError: (error: unknown) => void;
  readOnly?: boolean;
}

export default memo(function MessageItem({ message, senderName, mine, onError, readOnly = false }: Props) {
  const { t, i18n } = useTranslation();
  const pending = useRef(false);
  const [busy, setBusy] = useState(false);
  const act = (call: () => Promise<unknown>) => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    call()
      .catch(onError)
      .finally(() => {
        pending.current = false;
        setBusy(false);
      });
  };
  if (message.kind === "system") {
    return (
      <div className="system-message">
        {t(`system.${message.system_code}`, {
          ...systemParams(t, message.system_params),
          ...resetTime(message.system_params, i18n.language),
          defaultValue: message.system_code ?? "",
        })}
      </div>
    );
  }
  const time = new Date(message.created_at).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });
  return (
    <article className={mine ? "message mine" : "message"}>
      {message.task_name && <div className="muted task-run-label">{t("message.taskRun", { name: message.task_name })}</div>}
      {message.workflow_mark && <div className="muted task-run-label">{markLabel(message.workflow_mark, t)}</div>}
      {message.workflow_request && <div className="muted task-run-label">{t("message.workflowRequest")}</div>}
      <div className="message-meta">
        <strong>{senderName}</strong>
        <time dateTime={message.created_at}>{time}</time>
        {message.status === "queued" && <span className="badge">{t("message.queued")}</span>}
        {message.status === "cancelled" && <span className="badge">{t("message.cancelled")}</span>}
        {message.status === "failed" && <span className="badge">{t("message.failed")}</span>}
        {message.status === "retried" && <span className="badge">{t("message.retried")}</span>}
        {mine && message.status === "queued" && !readOnly && (
          <button className="link-button" disabled={busy} onClick={() => act(() => api.cancelMessage(message.id))}>
            {t("message.cancel")}
          </button>
        )}
      </div>
      {message.kind === "raw_fallback" ? (
        <details className="raw-output">
          <summary>{t("message.rawOutput")}</summary>
          <pre>{message.body}</pre>
        </details>
      ) : (
        splitPageElements(message.body).map((segment, i) =>
          segment.kind === "markdown" ? (
            <div className="markdown" key={i}>
              <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ a: MessageLink }}>
                {segment.text}
              </ReactMarkdown>
            </div>
          ) : (
            <details className="picked-element" key={i}>
              <summary>{elementSummary(segment.element)}</summary>
              <dl>
                <dt>{t("message.element.page")}</dt>
                <dd>
                  <MessageLink href={segment.element.url}>{segment.element.url}</MessageLink>
                </dd>
                <dt>{t("message.element.title")}</dt>
                <dd>{segment.element.title}</dd>
                <dt>{t("message.element.selector")}</dt>
                <dd>
                  <code>{segment.element.selector}</code>
                </dd>
                <dt>{t("message.element.text")}</dt>
                <dd>{segment.element.text}</dd>
                <dt>{t("message.element.html")}</dt>
                <dd>
                  <pre>{segment.element.html}</pre>
                </dd>
              </dl>
            </details>
          )
        )
      )}
      {message.attachments.length > 0 && (
        <ul className="attachments">
          {message.attachments.map((a, i) => (
            <li key={`${i}-${a.path}`} className="attachment">
              <span className="attachment-name">📎 {a.name}</span>
              {!opensPrograms(a.name) && (
                <button onClick={() => void api.openAttachment(message.id, i, false).catch(onError)}>
                  {t("message.open")}
                </button>
              )}
              <button onClick={() => void api.openAttachment(message.id, i, true).catch(onError)}>
                {t("message.reveal")}
              </button>
            </li>
          ))}
        </ul>
      )}
      {message.status === "interrupted" && (
        <div className="muted">
          {t("message.interrupted")}{" "}
          {!readOnly && (
            <button className="link-button" disabled={busy} onClick={() => act(() => api.retryMessage(message.id))}>
              {t("message.retry")}
            </button>
          )}
        </div>
      )}
    </article>
  );
});
