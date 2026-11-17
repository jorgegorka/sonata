import { useTranslation } from "react-i18next";
import type { ImportReport } from "../api";
import { groupWritten, itemName } from "../configImport";
import { errorMessage } from "../errors";

export default function ImportResult({ report, onClose }: { report: ImportReport; onClose: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="config-panel">
      {report.written.length === 0 && !report.error && <p className="muted">{t("config.nothing")}</p>}
      {groupWritten(report.written).map((group) => (
        <div key={group.kind}>
          <strong>{t(`config.${group.kind}`)}</strong>
          <ul>
            {group.items.map((w) => (
              <li key={w.name}>
                {t(w.replaced ? "config.replaced" : "config.added")}: {itemName(group.kind, w.name)}
              </li>
            ))}
          </ul>
        </div>
      ))}
      {report.backup && (
        <p className="muted">
          {report.backup.kept_at
            ? t("config.backupKept", { path: report.backup.kept_at })
            : t("config.backupTrash", { file: report.backup.file_name })}
        </p>
      )}
      {report.error && <p className="error-text">{t("config.failed", { error: errorMessage(t, report.error) })}</p>}
      <div className="config-actions">
        <button onClick={onClose}>{t("config.close")}</button>
      </div>
    </div>
  );
}
