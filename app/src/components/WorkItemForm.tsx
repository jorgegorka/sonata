import { useEffect, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import type { BotView, NewWorkItem, WorkItem, Workflow } from "../api";
import { useProjectApi } from "../projectContext";
import { errorMessage } from "../errors";
import { addFiles, fileName } from "../files";

export interface TaskCreation {
  id: number;
  parent?: WorkItem;
  executor?: { kind: "bot" | "workflow"; id: string };
}

interface Props {
  creation: TaskCreation;
  bots: BotView[];
  siblings: WorkItem[];
  onClose: () => void;
  onSaved: (item: WorkItem) => void;
}

export default function WorkItemForm({ creation, bots, siblings, onClose, onSaved }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const { parent } = creation;
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [workflowError, setWorkflowError] = useState<unknown>(null);
  const [attempt, setAttempt] = useState(0);
  const [title, setTitle] = useState("");
  const [instructions, setInstructions] = useState("");
  const [executor, setExecutor] = useState(() => creation.executor
    ? `${creation.executor.kind}:${creation.executor.id}` : `bot:${bots[0]?.id ?? ""}`);
  const [manual, setManual] = useState(false);
  const [dependencies, setDependencies] = useState<string[]>([]);
  const [files, setFiles] = useState<string[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [dragging, setDragging] = useState(false);
  const inFlight = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    let disposed = false;
    setWorkflowError(null);
    api.listWorkflows().then((items) => { if (!disposed) setWorkflows(items); })
      .catch((e) => { if (!disposed) setWorkflowError(e); });
    return () => { disposed = true; };
  }, [api, attempt]);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (disposed || inFlight.current) return;
      setDragging(payload.type !== "leave" && payload.type !== "drop");
      if (payload.type === "drop") setFiles((current) => addFiles(current, payload.paths));
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch((e) => { if (!disposed) setError(e); });
    return () => { disposed = true; unlisten?.(); };
  }, []);
  const selectedBot = bots.find((b) => executor === `bot:${b.id}`);
  const selectedWorkflow = workflows.find((w) => executor === `workflow:${w.id}`);
  const missingBot = selectedWorkflow?.def.steps.find((s) => !bots.some((b) => b.id === s.bot_id));
  const canSubmit = instructions.trim() !== "" && Boolean(selectedBot || selectedWorkflow) && !missingBot;
  async function pickFiles() {
    try {
      const picked = await open({ multiple: true, directory: false });
      if (picked && mounted.current && !inFlight.current) setFiles((current) => addFiles(current, Array.isArray(picked) ? picked : [picked]));
    } catch (e) { if (mounted.current) setError(e); }
  }
  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!canSubmit || inFlight.current) return;
    const input: NewWorkItem = {
      parent_id: parent?.id ?? null, title, instructions, manual_start: manual, depends_on: dependencies, files,
      executor: selectedBot ? { kind: "bot", handle: selectedBot.handle } : { kind: "workflow", name: selectedWorkflow!.name },
    };
    inFlight.current = true; setBusy(true); setError(null);
    try {
      const item = await api.createWorkItem(input);
      if (mounted.current) onSaved(item);
    } catch (e) { if (mounted.current) setError(e); }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  }
  return <aside className="drawer" aria-label={t(parent ? "work.addSubtask" : "board.newTask")}>
    <header className="drawer-header"><h2>{t(parent ? "work.addSubtask" : "board.newTask")}</h2><button onClick={onClose}>{t("board.close")}</button></header>
    <form className={`drawer-body work-form${dragging ? " dragging" : ""}`} onSubmit={(e) => void submit(e)}>
      {parent && <p>{t("work.parent", { title: parent.title })}</p>}
      <label>{t("work.title")}<input value={title} disabled={busy} onChange={(e) => setTitle(e.target.value)} /><span className="muted">{t("work.titleHint")}</span></label>
      <label>{t("board.pickBot")}<select value={executor} disabled={busy} onChange={(e) => setExecutor(e.target.value)}>
        {!selectedBot && !selectedWorkflow && <option value={executor}>{t("work.selectExecutor")}</option>}
        <optgroup label={t("board.pickBots")}>{bots.map((b) => <option key={b.id} value={`bot:${b.id}`}>{b.display_name}</option>)}</optgroup>
        <optgroup label={t("board.pickWorkflows")}>{workflows.map((w) => <option key={w.id} value={`workflow:${w.id}`}>{w.name}</option>)}</optgroup>
      </select></label>
      {workflowError != null && <div role="alert" className="error">{errorMessage(t, workflowError)} <button type="button" onClick={() => setAttempt((n) => n + 1)}>{t("work.retry")}</button></div>}
      {missingBot && <p className="error" role="alert">{t("errors.workflowStepNoBot", { step: missingBot.name })}</p>}
      <label>{t("work.instructions")}<textarea required autoFocus rows={8} value={instructions} disabled={busy} onChange={(e) => setInstructions(e.target.value)} /></label>
      <ul className="pending-files">{files.map((file) => <li key={file} className="chip">📎 {fileName(file)}<button type="button" disabled={busy} className="icon-button" aria-label={t("chat.removeFile", { name: fileName(file) })} onClick={() => setFiles((all) => all.filter((f) => f !== file))}>×</button></li>)}</ul>
      <button type="button" disabled={busy} onClick={() => void pickFiles()}>{t("chat.attach")}</button>
      {dragging && <p role="status">{t("chat.dropHere")}</p>}
      <label className="work-check"><input type="checkbox" disabled={busy} checked={manual} onChange={(e) => setManual(e.target.checked)} />{t("work.manual")}</label>
      {siblings.length > 0 && <fieldset disabled={busy}><legend>{t("work.dependenciesLabel")}</legend>{siblings.map((w) => <label key={w.id} className="work-check"><input type="checkbox" checked={dependencies.includes(w.id)} onChange={(e) => setDependencies((ids) => e.target.checked ? [...ids, w.id] : ids.filter((id) => id !== w.id))} />{w.title}</label>)}</fieldset>}
      {error != null && <div className="error" role="alert">{errorMessage(t, error)}</div>}
      <button type="submit" className="primary" disabled={busy || !canSubmit}>{t("work.create")}</button>
    </form>
  </aside>;
}
