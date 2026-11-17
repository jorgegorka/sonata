import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  api,
  type DependencyChoice,
  type ImportReport,
  type WorkflowFileEntry,
  type WorkflowImportItem,
  type WorkflowImportPreview,
  type WorkflowImportRequest,
} from "../api";
import { errorMessage } from "../errors";
import ImportResult from "./ImportResult";

interface Props {
  path: string;
  workflows: WorkflowFileEntry[];
  onBusy: (busy: boolean) => void;
  onClose: () => void;
  onImported: () => void;
}

export default function WorkflowImportPanel({ path, workflows, onBusy, onClose, onImported }: Props) {
  const { t } = useTranslation();
  const initial = (workflow: string): WorkflowImportRequest => ({
    workflow,
    workflow_choice: null,
    new_name: null,
    bots: {},
    skills: {},
  });
  const [request, setRequest] = useState(() => initial(workflows.length === 1 ? workflows[0].name : ""));
  const [view, setView] = useState<{ request: WorkflowImportRequest; preview: WorkflowImportPreview } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [changed, setChanged] = useState(false);
  const [applying, setApplying] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);
  const preview = view?.request === request ? view.preview : null;
  const displayed = view?.preview;

  useEffect(() => {
    if (!request.workflow) return;
    let active = true;
    setError(null);
    setChanged(false);
    onBusy(true);
    void api
      .previewWorkflowImport(path, request)
      .then((preview) => {
        if (active) setView({ request, preview });
      })
      .catch((err: unknown) => {
        if (active) setError(errorMessage(t, err));
      })
      .finally(() => {
        if (active) onBusy(false);
      });
    return () => {
      active = false;
      onBusy(false);
    };
  }, [path, request, onBusy, t]);

  async function apply() {
    if (!preview?.can_import || applying) return;
    setApplying(true);
    setError(null);
    onBusy(true);
    try {
      const result = await api.importWorkflowConfig(path, request, preview.token);
      if (result.outcome === "done") {
        setReport(result.report);
        onImported();
      } else {
        setView({ request, preview: result.preview });
        setChanged(result.outcome === "changed");
      }
    } catch (err) {
      setError(errorMessage(t, err));
    } finally {
      setApplying(false);
      onBusy(false);
    }
  }

  function choose(item: WorkflowImportItem, value: string) {
    if (item.kind === "workflow") {
      setRequest({
        ...request,
        workflow_choice: value === "copy" || value === "" ? null : (value as DependencyChoice),
        new_name: value === "copy" ? "" : null,
      });
    } else {
      const key = item.kind === "bot" ? "bots" : "skills";
      const choices = { ...request[key] };
      if (value) choices[item.name] = value as DependencyChoice;
      else delete choices[item.name];
      setRequest({ ...request, [key]: choices });
    }
  }

  if (report) return <ImportResult report={report} onClose={onClose} />;

  function itemRow(item: WorkflowImportItem) {
    const name = item.kind === "bot" ? `@${item.name}` : item.name;
    const isWorkflow = item.kind === "workflow";
    const value = isWorkflow
      ? request.new_name !== null
        ? "copy"
        : (request.workflow_choice ?? "")
      : ((item.kind === "bot" ? request.bots : request.skills)[item.name] ?? "");
    const hasChoice =
      isWorkflow || item.status.kind === "different" || (item.status.kind === "invalid" && item.can_keep);
    return (
      <li key={`${item.kind}:${item.name}`} className="workflow-import-item">
        <div>
          <strong>{name}</strong> <span className="muted">{t(`workflowTransfer.actions.${item.action}`)}</span>
        </div>
        {item.target && item.target !== item.name && (
          <span className="muted">
            {t("config.as", { name: item.kind === "bot" ? `@${item.target}` : item.target })}
          </span>
        )}
        {item.required_by.length > 0 && (
          <span className="muted">{t("workflowTransfer.requiredBy", { names: item.required_by.join(", ") })}</span>
        )}
        {hasChoice && (
          <label>
            <span>{t("workflowTransfer.choice", { name })}</span>
            <select
              disabled={applying}
              aria-label={t("workflowTransfer.choice", { name })}
              value={value}
              onChange={(e) => choose(item, e.target.value)}
            >
              <option value="">
                {item.status.kind === "unchanged"
                  ? t("workflowTransfer.actions.reuse")
                  : item.status.kind === "new"
                    ? t("workflowTransfer.actions.add")
                    : t("workflowTransfer.choose")}
              </option>
              {item.can_keep && <option value="keep_local">{t("workflowTransfer.keep")}</option>}
              {(item.can_keep || item.status.kind === "different") && (
                <option value="replace" disabled={item.status.kind === "invalid" && request.new_name === null}>
                  {t("workflowTransfer.replace")}
                </option>
              )}
              {isWorkflow && <option value="copy">{t("workflowTransfer.copy")}</option>}
            </select>
          </label>
        )}
        {isWorkflow && request.new_name !== null && (
          <label>
            {t("workflowTransfer.newName")}
            <input
              autoFocus
              value={request.new_name}
              disabled={applying}
              onChange={(e) => setRequest({ ...request, new_name: e.target.value })}
            />
          </label>
        )}
        {item.status.kind === "invalid" && (
          <span className={item.action === "reuse" ? "muted" : "error-text"}>
            {t("workflowTransfer.invalidIncoming", { error: errorMessage(t, item.status) })}
          </span>
        )}
        {(item.changed_fields.length > 0 || item.changed_files.length > 0) && (
          <details>
            <summary>{t("workflowTransfer.changes")}</summary>
            <ul>
              {item.changed_fields.map((field) => (
                <li key={field}>{t(`workflowTransfer.fields.${field}`)}</li>
              ))}
            </ul>
            {item.changed_files.length > 0 && (
              <>
                <span>{t("workflowTransfer.files")}</span>
                <ul>
                  {item.changed_files.map((file) => (
                    <li key={file}>
                      <code>{file}</code>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </details>
        )}
        {item.used_by_bots.length > 0 && (
          <span className="muted">
            {t("workflowTransfer.usedByBots", { names: item.used_by_bots.map((b) => `@${b}`).join(", ") })}
          </span>
        )}
        {item.used_by_workflows.length > 0 && (
          <span className="muted">
            {t("workflowTransfer.usedByWorkflows", { names: item.used_by_workflows.join(", ") })}
          </span>
        )}
      </li>
    );
  }

  return (
    <section
      className="config-panel workflow-import"
      aria-label={t("workflowTransfer.title")}
      aria-busy={(!!request.workflow && !preview && !error) || applying}
    >
      <h2>{t("workflowTransfer.title")}</h2>
      <p className="muted">{path.split("/").pop()}</p>
      {workflows.length > 1 && (
        <label>
          {t("workflowTransfer.selectWorkflow")}
          <select
            disabled={applying}
            value={request.workflow}
            onChange={(e) => {
              setView(null);
              setRequest(initial(e.target.value));
            }}
          >
            <option value="">{t("workflowTransfer.chooseWorkflow")}</option>
            {workflows.map((w) => (
              <option key={w.name} value={w.name}>
                {w.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {changed && (
        <p role="alert" className="error-text">
          {t("config.changed")}
        </p>
      )}
      {error && (
        <p role="alert" className="error-text">
          {error}
        </p>
      )}
      {request.workflow && !preview && !error && <p role="status">{t("workflowTransfer.loading")}</p>}
      {displayed && (
        <>
          {(["workflow", "bot", "skill"] as const).map((kind) => {
            const items = displayed.items.filter((item) => item.kind === kind);
            return (
              items.length > 0 && (
                <div key={kind}>
                  <h3>{t(`config.${kind === "workflow" ? "workflows" : kind === "bot" ? "bots" : "skills"}`)}</h3>
                  <ul className="workflow-import-items">{items.map(itemRow)}</ul>
                </div>
              )
            );
          })}
          <p>
            {t("workflowTransfer.summary", {
              addCount: displayed.items.filter((i) => i.action === "add").length,
              replaceCount: displayed.items.filter((i) => i.action === "replace").length,
              reuseCount: displayed.items.filter((i) => i.action === "reuse").length,
            })}
          </p>
          {preview && !preview.can_import && <p className="error-text">{t("workflowTransfer.resolve")}</p>}
          <p className="muted">{t("workflowTransfer.shared")}</p>
          <p className="muted">{t("config.trust")}</p>
        </>
      )}
      <div className="config-actions">
        <button className="primary" disabled={!preview?.can_import || applying} onClick={() => void apply()}>
          {t("workflowTransfer.apply")}
        </button>
        <button disabled={applying} onClick={onClose}>
          {t("config.cancel")}
        </button>
      </div>
    </section>
  );
}
