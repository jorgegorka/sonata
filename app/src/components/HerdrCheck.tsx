import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type HerdrStatus } from "../api";

export default function HerdrCheck({ onError }: { onError: (error: unknown) => void }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<HerdrStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .herdrStatus()
      .then((s) => {
        if (!cancelled) setStatus(s);
      })
      .catch(onError);
    return () => {
      cancelled = true;
    };
  }, [onError]);

  return (
    <div className={`check-row ${status?.error ? "missing" : status ? "ready" : ""}`}>
      <strong>{t("tools.herdr")}</strong>
      {status === null ? (
        <span className="muted">{t("tools.checking")}</span>
      ) : status.error ? (
        <span className="error-text">{t("tools.herdrFailed", { error: status.error })}</span>
      ) : (
        <span>{t("tools.herdrReady", { version: status.version, protocol: status.protocol })}</span>
      )}
    </div>
  );
}
