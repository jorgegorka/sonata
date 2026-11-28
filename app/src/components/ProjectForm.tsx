import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type Project, type ProjectInput, type Provider } from "../api";
import { PROVIDERS } from "../botForm";
import { errorMessage } from "../errors";
import { Button, Dialog, DialogHeader, Field, Icon } from "./ui";
import { useFormGuard } from "./useFormGuard";

export default function ProjectForm({ project, onSaved, onClose }: { project?: Project; onSaved: (project: Project) => void; onClose: () => void }) {
  const { t } = useTranslation();
  const id = useId();
  const [draft, setDraft] = useState<ProjectInput>(() => ({ name: project?.name ?? "", default_directory: project?.default_directory ?? null, default_provider: project?.default_provider ?? "claude" }));
  const initial = useRef(draft);
  const guard = useFormGuard(JSON.stringify(initial.current) !== JSON.stringify(draft), onClose);
  const mounted = useRef(true);
  const picking = useRef(false);
  const [pickerBusy, setPickerBusy] = useState(false);
  const [pickerError, setPickerError] = useState<string | null>(null);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const locked = guard.locked || pickerBusy;
  const close = () => { if (!picking.current) void guard.requestClose(); };
  async function choose() {
    if (locked || picking.current) return;
    picking.current = true; setPickerBusy(true); setPickerError(null);
    try {
      const picked = await open({ directory: true, multiple: false, defaultPath: draft.default_directory ?? undefined });
      if (mounted.current && typeof picked === "string") setDraft((current) => ({ ...current, default_directory: picked }));
    } catch (e) { if (mounted.current) setPickerError(errorMessage(t, e)); }
    finally { picking.current = false; if (mounted.current) setPickerBusy(false); }
  }
  function submit(event: FormEvent) {
    event.preventDefault();
    if (locked || picking.current || !draft.name.trim() || (!project && !draft.default_directory)) return;
    const input = { ...draft, name: draft.name.trim() };
    void guard.save(() => project ? api.updateProject(project.id, input) : api.createProject(input), onSaved);
  }
  return <Dialog labelledBy={`${id}-heading`} onRequestClose={close} submitOnShortcut>
    <form className="dialog-form" onSubmit={submit} aria-busy={guard.busy}>
      <DialogHeader id={`${id}-heading`} title={t(project ? "projects.edit" : "projects.add")} icon="folder" locked={locked} onClose={close} />
      <div className="dialog-body form-stack">
        <Field label={t("projects.name")} className="title-field"><input className="editor-title" data-autofocus required placeholder={t("projects.name")} disabled={locked} value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} /></Field>
        <div className="form-field"><span id={`${id}-directory`} className="field-label">{t("projects.directory")}</span>
          <div className="directory-control"><span className="path">{draft.default_directory ?? t("setup.projectNone")}</span><Button disabled={locked} aria-describedby={`${id}-directory`} onClick={() => void choose()}><Icon name="folder" />{t("setup.chooseCheckout")}</Button></div>
        </div>
        <Field label={t("projects.provider")} hint={t("projects.defaultsHint")}><select value={draft.default_provider} disabled={locked} onChange={(e) => setDraft({ ...draft, default_provider: e.target.value as Provider })}>{PROVIDERS.map((provider) => <option key={provider} value={provider}>{t(`providers.${provider}`)}</option>)}</select></Field>
        {(guard.error || pickerError) && <div className="error" role="alert">{guard.error || pickerError}</div>}
      </div>
      <footer className="dialog-footer actions"><Button disabled={locked} onClick={close}>{t("createBot.cancel")}</Button><Button type="submit" variant="primary" disabled={locked || !draft.name.trim() || (!project && !draft.default_directory)}>{t(guard.busy ? "botForm.saving" : "botForm.save")}</Button></footer>
    </form>
  </Dialog>;
}
