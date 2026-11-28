import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, type Project } from "../api";
import ProjectForm from "./ProjectForm";
import { Button, PageHeader, Icon, StatusBadge } from "./ui";

interface Props {
  projects: Project[];
  adding: boolean;
  onChanged: (selected?: string) => void;
  onClose: () => void;
  onError: (e: unknown) => void;
}
export default function ProjectsView({ projects, adding, onChanged, onClose, onError }: Props) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<Project | "new" | null>(adding ? "new" : null);
  const [busy, setBusy] = useState(false);
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
  return <section className="page projects-view">
    <PageHeader title={t("projects.manage")} actions={<><Button variant="ghost" onClick={onClose}>{t("projects.back")}</Button><Button variant="primary" onClick={() => setEditing("new")}><Icon name="plus" />{t("projects.add")}</Button></>} />
    {editing && <ProjectForm key={editing === "new" ? "new" : editing.id} project={editing === "new" ? undefined : editing}
      onClose={() => setEditing(null)} onSaved={(saved) => { setEditing(null); onChanged(saved.id); }} />}
    <ul className="project-list">{projects.map((p) => <li key={p.id}>
      <div><strong>{p.name}</strong>{p.archived_at && <StatusBadge>{t("projects.archived")}</StatusBadge>}
        <p className="path">{p.default_directory ?? t("setup.projectNone")}</p><span className="muted">{t(`providers.${p.default_provider}`)}</span></div>
      <div className="actions">{p.archived_at ? <button disabled={busy} onClick={() => void restore(p)}>{t("projects.restore")}</button> : <>
        <button disabled={busy} onClick={() => setEditing(p)}>{t("projects.edit")}</button>
        <button disabled={busy} onClick={() => void archive(p)}>{t("projects.archive")}</button>
      </>}</div>
    </li>)}</ul>
    <p className="muted">{t("projects.archiveHint")}</p>
  </section>;
}
