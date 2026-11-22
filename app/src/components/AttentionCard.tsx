import { useProjectApi } from "../projectContext";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { type SessionView } from "../api";
import TerminalView from "./TerminalView";

interface Props {
  session: SessionView;
  onError: (error: unknown) => void;
}

export default function AttentionCard({ session, onError }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [running, setRunning] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);

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

  return (
    <div className="attention" role="alert">
      <strong>{t("attention.title")}</strong>
      <span className="muted">{running === false ? t("attention.notRunning") : t("attention.dialog")}</span>
      <TerminalView sessionId={session.id} onRunning={setRunning} noteWhenStopped={false} />
      <div className="actions">
        {running !== false && (
          <button disabled={busy} onClick={() => void act(() => api.answerDialog(session.id, "continue"))}>
            {t("attention.continue")}
          </button>
        )}
        {running !== false && (
          <button disabled={busy} onClick={() => void act(() => api.answerDialog(session.id, "cancel"))}>
            {t("attention.cancel")}
          </button>
        )}
        <button className="primary" disabled={busy} onClick={() => void act(() => api.restartSession(session.id))}>
          {t("attention.restart")}
        </button>
      </div>
    </div>
  );
}
