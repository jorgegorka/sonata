import { Dialog } from "./ui";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { updatesApi, updateIsVisible, type UpdateState } from "../updates";

export default function UpdatePrompt({ state, onError }: { state: UpdateState | null; onError: (error: unknown) => void }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [guide, setGuide] = useState(false);
  const visible = updateIsVisible(state);
  useEffect(() => { if (!visible) setGuide(false); }, [visible]);
  if (!visible || !state) return null;
  const phase = state.phase;
  const guided = state.installation === "arch" || state.installation === "extracted";
  const locked = busy || ["downloading", "installing", "install_failed"].includes(phase);
  async function act(action: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    try { await action(); } catch (error) { onError(error); } finally { setBusy(false); }
  }
  return (
    <Dialog className="update-dialog" labelledBy="update-title" onRequestClose={() => { if (!locked) void act(updatesApi.dismiss); }}>
      <div className="dialog-body">
      <h2 id="update-title">{t("updates.available", { version: state.version })}</h2>
      {phase === "available" && <p>{t("updates.offer")}</p>}
      {phase === "downloading" && <div role="status" aria-live="polite">
        <p>{t("updates.downloading")}</p>
        <progress aria-label={t("updates.downloading")} max={state.total || undefined}
          value={state.total ? state.downloaded : undefined} />
      </div>}
      {phase === "ready" && <><p>{t("updates.ready")}</p><p>{t("updates.restartHint")}</p></>}
      {phase === "installing" && <p role="status">{t("updates.installing")}</p>}
      {phase === "download_failed" && <p role="alert">{t("updates.downloadFailed")}</p>}
      {phase === "install_failed" && <p role="alert">{t("updates.installFailed")}</p>}
      {guided && guide && <p>{t(state.installation === "arch" ? "updates.archHint" : "updates.extractedHint", { version: state.version })}</p>}
      </div>
      <div className="dialog-footer update-actions">
        {phase === "available" && !guided && <button className="primary" disabled={busy} onClick={() => void act(updatesApi.download)}>{t("updates.download")}</button>}
        {phase === "available" && guided && !guide && <button className="primary" disabled={busy} onClick={() => setGuide(true)}>{t("updates.instructions")}</button>}
        {phase === "available" && guided && guide && <button className="primary" disabled={busy} onClick={() => void act(() => updatesApi.openLink(true))}>{t("updates.download")}</button>}
        {phase === "download_failed" && <button className="primary" disabled={busy} onClick={() => void act(updatesApi.download)}>{t("updates.retry")}</button>}
        {phase === "ready" && <button className="primary" disabled={busy} onClick={() => void act(updatesApi.install)}>{t("updates.restartInstall")}</button>}
        {phase === "install_failed" && <button className="primary" disabled={busy} onClick={() => void act(updatesApi.restart)}>{t("updates.restart")}</button>}
        {!["installing", "install_failed"].includes(phase) && <button disabled={busy} onClick={() => void act(() => updatesApi.openLink())}>{t("updates.notes")}</button>}
        {!locked && <button onClick={() => void act(updatesApi.dismiss)}>{t("updates.later")}</button>}
      </div>
    </Dialog>
  );
}
