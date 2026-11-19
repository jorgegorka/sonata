import { useState, type FormEvent, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { api, type BotView, type SentMessage, type Workflow, type WorkflowFileEntry } from "../api";
import { errorMessage } from "../errors";
import { workflowExportFileName } from "../configImport";
import WorkflowImportPanel from "./WorkflowImportPanel";

interface Props {
  workflows: Workflow[];
  bots: BotView[];
  onNew: () => void;
  onImported: () => void;
  onEdit: (workflowId: string) => void;
  onRun: (workflowId: string) => void;
  onRequested: (sent: SentMessage) => void;
}

export default function WorkflowsView({ workflows, bots, onNew, onEdit, onRun, onRequested, onImported }: Props) {
  const { t } = useTranslation();
  const [error, setError] = useState<string | null>(null);
  const [configBusy, setConfigBusy] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  const [importing, setImporting] = useState<{ path: string; workflows: WorkflowFileEntry[]; key: number } | null>(
    null,
  );
  const [picks, setPicks] = useState(0);
  const [description, setDescription] = useState("");
  const [sending, setSending] = useState(false);
  const [requestError, setRequestError] = useState<string | null>(null);

  const filters = () => [{ name: t("config.fileFilter"), extensions: ["yaml", "yml"] }];

  async function exportWorkflow(workflow: Workflow) {
    setConfigBusy(true);
    setError(null);
    try {
      const path = await save({ defaultPath: workflowExportFileName(workflow.name, new Date()), filters: filters() });
      if (typeof path !== "string") return;
      const summary = await api.exportWorkflowConfig(workflow.id, path);
      setImporting(null);
      setExported(t("config.exported", { file: path.split("/").pop(), ...summary }));
    } catch (err) {
      setError(errorMessage(t, err));
    } finally {
      setConfigBusy(false);
    }
  }

  async function importWorkflow() {
    setConfigBusy(true);
    setError(null);
    try {
      const path = await open({ multiple: false, directory: false, filters: filters() });
      if (typeof path !== "string") return;
      const entries = await api.workflowsInConfig(path);
      if (entries.length === 0) {
        setError(t("workflowTransfer.noWorkflows"));
        return;
      }
      setExported(null);
      setPicks(picks + 1);
      setImporting({ path, workflows: entries, key: picks + 1 });
    } catch (err) {
      setError(errorMessage(t, err));
    } finally {
      setConfigBusy(false);
    }
  }

  async function request(e?: FormEvent) {
    e?.preventDefault();
    if (sending || description.trim() === "") return;
    setRequestError(null);
    setSending(true);
    try {
      const sent = await api.requestWorkflow(description);
      setDescription("");
      onRequested(sent);
    } catch (err) {
      setRequestError(errorMessage(t, err));
    } finally {
      setSending(false);
    }
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void request();
    }
  }

  async function remove(workflow: Workflow) {
    const confirmed = await ask(t("workflows.deleteConfirm", { name: workflow.name }), {
      title: t("workflows.deleteTitle"),
      kind: "warning",
      okLabel: t("workflows.delete"),
      cancelLabel: t("workflowEditor.cancel"),
    });
    if (!confirmed) return;
    setError(null);
    try {
      await api.deleteWorkflow(workflow.id);
    } catch (e) {
      setError(errorMessage(t, e));
    }
  }

  return (
    <div className="settings">
      <h1>{t("workflows.title")}</h1>
      <p className="muted">{t("workflows.intro")}</p>
      <div className="actions start">
        <button className="primary" onClick={onNew}>
          {t("workflows.new")}
        </button>
        <button disabled={configBusy} onClick={() => void importWorkflow()}>
          {t("workflowTransfer.import")}
        </button>
      </div>
      {error && (
        <div className="error" role="alert" onClick={() => setError(null)}>
          {error}
        </div>
      )}
      {exported && (
        <p role="status" className="muted">
          {exported}
        </p>
      )}
      {importing && (
        <WorkflowImportPanel
          key={importing.key}
          path={importing.path}
          workflows={importing.workflows}
          onBusy={setConfigBusy}
          onClose={() => setImporting(null)}
          onImported={onImported}
        />
      )}
      {workflows.length === 0 && <p className="muted">{t("workflows.empty")}</p>}
      <ul className="skills-list">
        {workflows.map((workflow) => {
          const botless = workflow.def.steps.filter((s) => !bots.some((b) => b.id === s.bot_id));
          return (
            <li key={workflow.id} className="task-row">
              <strong>{workflow.name}</strong>
              {workflow.when_to_use && <span>{workflow.when_to_use}</span>}
              <span className="muted">{workflow.def.steps.map((s) => s.name).join(" → ")}</span>
              {botless.map((s) => (
                <span key={s.id} className="error-text">
                  {t("errors.workflowStepNoBot", { step: s.name })}
                </span>
              ))}
              <div className="actions start">
                <button onClick={() => onRun(workflow.id)}>{t("workflows.run")}</button>
                <button onClick={() => onEdit(workflow.id)}>{t("workflows.edit")}</button>
                <button disabled={configBusy} onClick={() => void exportWorkflow(workflow)}>
                  {t("config.export")}
                </button>
                <button className="danger" disabled={configBusy} onClick={() => void remove(workflow)}>
                  {t("workflows.delete")}
                </button>
              </div>
            </li>
          );
        })}
      </ul>
      <form className="workflow-request" onSubmit={(e) => void request(e)}>
        <h2>{t("workflows.describeTitle")}</h2>
        <p className="muted">{t("workflows.describeHint")}</p>
        <textarea
          aria-label={t("workflows.describeTitle")}
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={t("workflows.describePlaceholder")}
          rows={5}
        />
        {requestError && (
          <div className="error" role="alert" onClick={() => setRequestError(null)}>
            {requestError}
          </div>
        )}
        <div className="actions">
          <button type="submit" className="primary" disabled={sending || description.trim() === ""}>
            {t("workflows.describeSend")}
          </button>
        </div>
      </form>
    </div>
  );
}
