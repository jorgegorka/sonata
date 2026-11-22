import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, type Project, type ProjectInput, type Provider } from "../api";
import { PROVIDERS } from "../botForm";

interface Props {
  projects: Project[];
  adding: boolean;
  onChanged: (selected?: string) => void;
  onClose: () => void;
  onError: (e: unknown) => void;
}
export default function ProjectsView({ projects, adding, onChanged, onClose, onError }: Props) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<Project | null>(null);
  const [form, setForm] = useState<ProjectInput | null>(adding ? { name: "", default_directory: null, default_provider: "claude" } : null);
  const [busy, setBusy] = useState(false);
  function edit(project: Project) {
    setEditing(project);
    setForm({ name: project.name, default_directory: project.default_directory, default_provider: project.default_provider });
  }
  async function choose() {
    try {
      const picked = await open({ directory: true, multiple: false, defaultPath: form?.default_directory ?? undefined });
      if (typeof picked === "string") setForm((f) => f && { ...f, default_directory: picked });
    } catch (e) { onError(e); }
  }
  async function save(e: FormEvent) {
    e.preventDefault();
    if (!form) return;
    setBusy(true);
    try {
      const saved = editing ? await api.updateProject(editing.id, form) : await api.createProject(form);
      onChanged(saved.id);
      setForm(null);
      setEditing(null);
    } catch (e) { onError(e); }
    finally { setBusy(false); }
  }
  async function archive(project: Project) {
    setBusy(true);
    try {
      if (!await ask(t("projects.archiveConfirm", { name: project.name }), { title: t("projects.archive"), kind: "warning" })) return;
      await api.archiveProject(project.id);
      onChanged();
    }
    catch (e) { onError(e); }
    finally { setBusy(false); }
  }
  async function restore(project: Project) {
    setBusy(true);
    try { await api.restoreProject(project.id); onChanged(project.id); }
    catch (e) { onError(e); }
    finally { setBusy(false); }
  }
  return <section className="projects-view">
    <div className="header-actions"><h1>{t("projects.manage")}</h1><button onClick={onClose}>{t("projects.back")}</button></div>
    {form ? <form className="create-bot" onSubmit={save}>
      <h2>{t(editing ? "projects.edit" : "projects.add")}</h2>
      <label>{t("projects.name")}<input autoFocus required value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} /></label>
      <label>{t("projects.directory")}</label>
      <p className="path">{form.default_directory ?? t("setup.projectNone")}</p>
      <button type="button" onClick={() => void choose()}>{t("setup.chooseCheckout")}</button>
      <label>{t("projects.provider")}<select value={form.default_provider} onChange={(e) => setForm({ ...form, default_provider: e.target.value as Provider })}>
        {PROVIDERS.map((p) => <option key={p} value={p}>{t(`providers.${p}`)}</option>)}
      </select></label>
      <p className="muted">{t("projects.defaultsHint")}</p>
      <div className="actions"><button type="button" disabled={busy} onClick={() => setForm(null)}>{t("createBot.cancel")}</button>
        <button className="primary" disabled={busy || !form.name.trim() || (!editing && !form.default_directory)}>{t("botForm.save")}</button></div>
    </form> : <button onClick={() => { setEditing(null); setForm({ name: "", default_directory: null, default_provider: "claude" }); }}>{t("projects.add")}</button>}
    <ul className="project-list">{projects.map((p) => <li key={p.id}>
      <div><strong>{p.name}</strong>{p.archived_at && <span className="muted"> · {t("projects.archived")}</span>}
        <p className="path">{p.default_directory ?? t("setup.projectNone")}</p><span className="muted">{t(`providers.${p.default_provider}`)}</span></div>
      <div className="actions">{p.archived_at ? <button disabled={busy} onClick={() => void restore(p)}>{t("projects.restore")}</button> : <>
        <button disabled={busy} onClick={() => edit(p)}>{t("projects.edit")}</button>
        <button disabled={busy} onClick={() => void archive(p)}>{t("projects.archive")}</button>
      </>}</div>
    </li>)}</ul>
    <p className="muted">{t("projects.archiveHint")}</p>
  </section>;
}
