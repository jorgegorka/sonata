import { useEffect, useId, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ask, open } from "@tauri-apps/plugin-dialog";
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
  const id = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  const description = useRef<HTMLTextAreaElement>(null);
  const propertyButton = useRef<HTMLButtonElement | null>(null);
  const [property, setProperty] = useState<"start" | "dependencies" | null>(null);
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [workflowError, setWorkflowError] = useState<unknown>(null);
  const [attempt, setAttempt] = useState(0);
  const [title, setTitle] = useState("");
  const [instructions, setInstructions] = useState("");
  const [initialExecutor] = useState(() => creation.executor
    ? `${creation.executor.kind}:${creation.executor.id}` : `bot:${bots[0]?.id ?? ""}`);
  const [executor, setExecutor] = useState(initialExecutor);
  const [manual, setManual] = useState(false);
  const [dependencies, setDependencies] = useState<string[]>([]);
  const [files, setFiles] = useState<string[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [dragging, setDragging] = useState(false);
  const inFlight = useRef(false);
  const closing = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    const previous = document.activeElement;
    const element = dialog.current!;
    element.showModal();
    description.current?.focus();
    return () => {
      element.close();
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, []);
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
      if (disposed || inFlight.current || closing.current) return;
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
  const locked = busy || confirming;
  const dirty = title !== "" || instructions !== "" || files.length > 0 || manual
    || dependencies.length > 0 || executor !== initialExecutor;
  async function requestClose() {
    if (inFlight.current || closing.current) return;
    if (!dirty) { onClose(); return; }
    closing.current = true; setConfirming(true);
    try {
      const discard = await ask(t("work.discardMessage"), {
        title: t("work.discardTitle"), kind: "warning",
        okLabel: t("work.discard"), cancelLabel: t("work.keepEditing"),
      });
      if (discard && mounted.current) onClose();
    } catch (e) { if (mounted.current) setError(e); }
    finally { closing.current = false; if (mounted.current) setConfirming(false); }
  }
  function closeProperty() {
    setProperty(null);
    propertyButton.current?.focus();
  }
  function keys(e: KeyboardEvent<HTMLDialogElement>) {
    if (e.key === "Escape" && property) {
      e.preventDefault(); e.stopPropagation(); closeProperty();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!locked && canSubmit) dialog.current?.querySelector("form")?.requestSubmit();
    }
  }
  async function pickFiles() {
    if (inFlight.current || closing.current) return;
    try {
      const picked = await open({ multiple: true, directory: false });
      if (picked && mounted.current && !inFlight.current && !closing.current) setFiles((current) => addFiles(current, Array.isArray(picked) ? picked : [picked]));
    } catch (e) { if (mounted.current) setError(e); }
  }
  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!canSubmit || inFlight.current || closing.current) return;
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
  return <dialog ref={dialog} className={`task-dialog${dragging ? " dragging" : ""}`} aria-labelledby={`${id}-heading`}
    onKeyDown={keys} onCancel={(e) => { e.preventDefault(); if (property) closeProperty(); else void requestClose(); }}>
    <form className="work-form" onSubmit={(e) => void submit(e)} aria-busy={busy}>
      <header className="task-dialog-header">
        <div className="task-dialog-heading"><Glyph kind="task" /><h2 id={`${id}-heading`}>{t(parent ? "work.addSubtask" : "board.newTask")}</h2></div>
        <button type="button" className="task-dialog-close" disabled={locked} aria-label={t("board.close")} title={t("board.close")} onClick={() => void requestClose()}><Glyph kind="close" /></button>
      </header>
      <div className="task-dialog-body">
        {parent && <p className="task-parent" title={parent.title}>{t("work.parent", { title: parent.title })}</p>}
        <label className="sr-only" htmlFor={`${id}-title`}>{t("work.title")}</label>
        <input id={`${id}-title`} className="task-title-input" placeholder={t("work.title")} aria-describedby={`${id}-hint`} value={title} disabled={locked} onChange={(e) => setTitle(e.target.value)} />
        <span id={`${id}-hint`} className="sr-only">{t("work.titleHint")}</span>
        <label className="sr-only" htmlFor={`${id}-description`}>{t("work.instructions")}</label>
        <textarea ref={description} id={`${id}-description`} className="task-description" placeholder={t("work.descriptionPlaceholder")} required rows={6} value={instructions} disabled={locked} onChange={(e) => setInstructions(e.target.value)} />
        {files.length > 0 && <ul className="pending-files task-files">{files.map((file) => <li key={file} className="chip"><Glyph kind="attach" /><span title={fileName(file)}>{fileName(file)}</span><button type="button" disabled={locked} className="icon-button" aria-label={t("chat.removeFile", { name: fileName(file) })} onClick={() => setFiles((all) => all.filter((f) => f !== file))}>×</button></li>)}</ul>}
        <div className="task-properties">
          <label className="task-executor"><Glyph kind={selectedWorkflow ? "workflow" : "bot"} /><span className="sr-only">{t("work.selectExecutor")}</span>
            <select value={executor} disabled={locked} title={selectedBot?.display_name ?? selectedWorkflow?.name ?? t("work.selectExecutor")} onChange={(e) => setExecutor(e.target.value)}>
              {!selectedBot && !selectedWorkflow && <option value={executor}>{t("work.selectExecutor")}</option>}
              <optgroup label={t("board.pickBots")}>{bots.map((b) => <option key={b.id} value={`bot:${b.id}`}>{b.display_name}</option>)}</optgroup>
              <optgroup label={t("board.pickWorkflows")}>{workflows.map((w) => <option key={w.id} value={`workflow:${w.id}`}>{w.name}</option>)}</optgroup>
            </select>
          </label>
          <button type="button" className="task-property" disabled={locked} aria-expanded={property === "start"} aria-controls={`${id}-start`} onClick={(e) => { propertyButton.current = e.currentTarget; setProperty(property === "start" ? null : "start"); }}><Glyph kind={manual ? "pause" : "play"} />{t(manual ? "work.manual" : "work.automaticStart")}</button>
          {siblings.length > 0 && <button type="button" className="task-property" disabled={locked} aria-expanded={property === "dependencies"} aria-controls={`${id}-dependencies`} onClick={(e) => { propertyButton.current = e.currentTarget; setProperty(property === "dependencies" ? null : "dependencies"); }}><Glyph kind="dependency" />{t("work.dependenciesLabel")}{dependencies.length > 0 && <span className="task-property-count">{dependencies.length}</span>}</button>}
        </div>
        {property === "start" && <fieldset id={`${id}-start`} className="task-property-panel" disabled={locked}><legend className="sr-only">{t("work.startMode")}</legend>
          <label className="work-check"><input type="radio" name={`${id}-start-mode`} checked={!manual} onChange={() => setManual(false)} />{t("work.automaticStart")}</label>
          <label className="work-check"><input type="radio" name={`${id}-start-mode`} checked={manual} onChange={() => setManual(true)} />{t("work.manual")}</label>
        </fieldset>}
        {property === "dependencies" && <fieldset id={`${id}-dependencies`} className="task-property-panel" disabled={locked}><legend className="sr-only">{t("work.dependenciesLabel")}</legend>{siblings.map((w) => <label key={w.id} className="work-check"><input type="checkbox" checked={dependencies.includes(w.id)} onChange={(e) => setDependencies((ids) => e.target.checked ? [...ids, w.id] : ids.filter((value) => value !== w.id))} /><span>{w.title}</span></label>)}</fieldset>}
        {workflowError != null && <div role="alert" className="error">{errorMessage(t, workflowError)} <button type="button" disabled={locked} onClick={() => setAttempt((n) => n + 1)}>{t("work.retry")}</button></div>}
        {missingBot && <p className="error" role="alert">{t("errors.workflowStepNoBot", { step: missingBot.name })}</p>}
        {error != null && <div className="error" role="alert">{errorMessage(t, error)}</div>}
      </div>
      {dragging && <div className="task-drop-hint" role="status"><Glyph kind="attach" />{t("chat.dropHere")}</div>}
      <footer className="task-dialog-footer">
        <button type="button" className="task-attach" disabled={locked} onClick={() => void pickFiles()}><Glyph kind="attach" />{t("chat.attach")}</button>
        <div className="task-submit"><span className="task-shortcut" aria-hidden="true">{navigator.platform.includes("Mac") ? "⌘" : "Ctrl"} ↵</span><button type="submit" className="primary" disabled={locked || !canSubmit}>{t(busy ? "work.creating" : "work.create")}</button></div>
      </footer>
    </form>
  </dialog>;
}

function Glyph({ kind }: { kind: "task" | "close" | "attach" | "bot" | "workflow" | "play" | "pause" | "dependency" }) {
  const paths = {
    task: "M5 3h10v14H5z M8 7h4 M8 10h4 M8 13h2",
    close: "M5 5l10 10 M15 5L5 15",
    attach: "M7 10l5-5a3 3 0 014 4l-7 7a4 4 0 01-6-6l7-7 M6 11l6-6",
    bot: "M4 7h12v9H4z M10 3v4 M7 11h.01 M13 11h.01 M7 14h6",
    workflow: "M3 3h5v5H3z M12 12h5v5h-5z M8 5h6v7 M5 8v6h7",
    play: "M6 4l10 6-10 6z",
    pause: "M6 4v12 M14 4v12",
    dependency: "M3 5h5v5H3z M12 10h5v5h-5z M8 7h7v3",
  };
  return <svg className="task-glyph" width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[kind]} /></svg>;
}
