import { useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { api, type SessionView } from "../api";
import { renamedTo } from "../state";

interface Props {
  session: SessionView;
  label: string;
  onError: (error: unknown) => void;
}

export default function SessionName({ session, label, onError }: Props) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);

  if (!editing) {
    return (
      <button className="session-name" title={t("chat.rename")} onClick={() => setEditing(true)}>
        {label}
      </button>
    );
  }
  return <SessionNameInput session={session} label={label} onDone={() => setEditing(false)} onError={onError} />;
}

export function SessionNameInput({ session, label, onDone, onError }: Props & { onDone: () => void }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(session.title ?? "");
  const cancelled = useRef(false);

  function finish() {
    const name = cancelled.current ? null : renamedTo(draft, session.title);
    onDone();
    if (name !== null) api.renameSession(session.id, name).catch(onError);
  }

  function onKey(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      cancelled.current = true;
      e.currentTarget.blur();
    }
  }

  return (
    <input
      className="session-name-input"
      type="text"
      autoFocus
      maxLength={60}
      aria-label={t("chat.rename")}
      placeholder={label}
      value={draft}
      spellCheck={false}
      onChange={(e) => setDraft(e.target.value)}
      onFocus={(e) => e.target.select()}
      onBlur={finish}
      onKeyDown={onKey}
    />
  );
}
