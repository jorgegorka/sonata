import ImportResult from "./ImportResult";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type ImportMode, type ImportPreview, type ImportReport, type PlannedItem } from "../api";
import { groupPreview, itemName, writes, type ConfigKind } from "../configImport";
import { errorMessage } from "../errors";

interface Props {
  path: string;
  preview: ImportPreview;
  busy: boolean;
  onBusy: (busy: boolean) => void;
  onClose: () => void;
  onImported: () => void;
  onError: (e: unknown) => void;
}

export default function ImportPanel({ path, preview: first, busy, onBusy, onClose, onImported, onError }: Props) {
  const { t } = useTranslation();
  const [preview, setPreview] = useState(first);
  const [changed, setChanged] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);

  async function run(mode: ImportMode) {
    onBusy(true);
    try {
      const result = await api.importConfig(path, mode, preview.token);
      if (result.outcome === "changed") {
        setPreview(result.preview);
        setChanged(true);
      } else {
        setReport(result.report);
        onImported();
      }
    } catch (err) {
      onError(err);
    } finally {
      onBusy(false);
    }
  }

  if (report) return <ImportResult report={report} onClose={onClose} />;

  function renamed(kind: ConfigKind, item: PlannedItem) {
    const other = item.target !== "" && item.target !== item.name;
    return other ? t("config.as", { name: itemName(kind, item.target) }) : undefined;
  }

  function reason(item: PlannedItem) {
    const { status } = item;
    return status.kind === "invalid" ? errorMessage(t, { code: status.code, params: status.params }) : undefined;
  }

  function line(kind: ConfigKind, item: PlannedItem, ...extras: (string | undefined)[]) {
    const extra = extras.filter(Boolean).join(" · ");
    return (
      <li key={item.name}>
        {itemName(kind, item.name)}
        {extra && <span className="muted"> {extra}</span>}
      </li>
    );
  }

  return (
    <div className="config-panel">
      {changed && <p className="error-text">{t("config.changed")}</p>}
      {groupPreview(preview).map((group) => (
        <div key={group.kind}>
          <strong>{t(`config.${group.kind}`)}</strong>
          {group.added.length > 0 && (
            <>
              <h3>{t("config.new")}</h3>
              <ul>{group.added.map((i) => line(group.kind, i, renamed(group.kind, i)))}</ul>
            </>
          )}
          {group.here.length > 0 && (
            <>
              <h3>{t("config.here")}</h3>
              <ul>
                {group.here.map((i) =>
                  line(
                    group.kind,
                    i,
                    renamed(group.kind, i),
                    t(i.status.kind === "different" ? "config.different" : "config.unchanged"),
                  ),
                )}
              </ul>
            </>
          )}
          {group.cant.length > 0 && (
            <>
              <h3>{t("config.cant")}</h3>
              <ul>{group.cant.map((i) => line(group.kind, i, reason(i)))}</ul>
            </>
          )}
        </div>
      ))}
      <p className="muted">{t("config.trust")}</p>
      <div className="config-actions">
        <button
          className="primary"
          disabled={busy || writes(preview, "add_new") === 0}
          onClick={() => void run("add_new")}
        >
          {t("config.addNew")}
        </button>
        <button disabled={busy || writes(preview, "add_and_replace") === 0} onClick={() => void run("add_and_replace")}>
          {t("config.addAndReplace")}
        </button>
        <button disabled={busy} onClick={onClose}>
          {t("config.cancel")}
        </button>
      </div>
    </div>
  );
}
