import { useProjectApi } from "../projectContext";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { type Skill, type SkillView } from "../api";
import { deletedAnyway } from "../botForm";
import { errorMessage } from "../errors";
import { canSave, EMPTY_SKILL, type SkillDraft } from "../skills";

interface Props {
  skill?: SkillView;
  onSaved: (skill: Skill) => void;
  onDeleted?: (skillId: string) => void;
  onCancel: () => void;
  onError: (error: unknown) => void;
}

export default function SkillForm({ skill, onSaved, onDeleted, onCancel, onError }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [draft, setDraft] = useState<SkillDraft>(() =>
    skill ? { name: skill.name, description: skill.description, body: "" } : EMPTY_SKILL,
  );
  const [loaded, setLoaded] = useState(!skill);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = <K extends keyof SkillDraft>(key: K, value: SkillDraft[K]) => setDraft((d) => ({ ...d, [key]: value }));

  const skillId = skill?.id;
  useEffect(() => {
    if (!skillId) return;
    let cancelled = false;
    api
      .getSkillBody(skillId)
      .then((body) => {
        if (cancelled) return;
        setDraft((d) => ({ ...d, body }));
        setLoaded(true);
      })
      .catch((e) => {
        if (!cancelled) onError(e);
      });
    return () => {
      cancelled = true;
    };
  }, [skillId, onError]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onSaved(skill ? await api.updateSkill(skill.id, draft) : await api.createSkill(draft));
    } catch (err) {
      setError(errorMessage(t, err));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!skill) return;
    const confirmed = await ask(t("skills.deleteConfirm", { name: skill.name }), {
      title: t("skills.deleteTitle"),
      kind: "warning",
      okLabel: t("skills.delete"),
      cancelLabel: t("skills.cancel"),
    });
    if (!confirmed) return;
    setBusy(true);
    try {
      await api.deleteSkill(skill.id);
      onDeleted?.(skill.id);
    } catch (err) {
      if (deletedAnyway(err)) {
        onDeleted?.(skill.id);
        onError(err);
        return;
      }
      setError(errorMessage(t, err));
      setBusy(false);
    }
  }

  const inUse = skill ? skill.used_by.map((h) => `@${h}`).join(", ") : "";
  return (
    <form className="create-bot" onSubmit={submit}>
      <h1>{skill ? t("skills.editTitle", { name: skill.name }) : t("skills.createTitle")}</h1>
      <p className="muted">{skill ? t("skills.folder", { slug: skill.slug }) : t("skills.folderHint")}</p>
      <label>
        {t("skills.name")}
        <input
          value={draft.name}
          onChange={(e) => set("name", e.target.value)}
          placeholder={t("skills.namePlaceholder")}
          maxLength={64}
          required
          autoFocus
        />
      </label>
      <label>
        {t("skills.description")}
        <textarea
          value={draft.description}
          onChange={(e) => set("description", e.target.value)}
          rows={2}
          maxLength={1024}
          required
        />
        <span className="muted">{t("skills.descriptionHint")}</span>
      </label>
      <label>
        {t("skills.body")}
        <textarea
          value={draft.body}
          onChange={(e) => set("body", e.target.value)}
          placeholder={loaded ? t("skills.bodyPlaceholder") : t("skills.loading")}
          disabled={!loaded}
          rows={14}
        />
      </label>
      {skill && skill.used_by.length > 0 && <p className="muted">{t("skills.appliesNext")}</p>}
      {error && (
        <div className="error" role="alert">
          {error}
        </div>
      )}
      <div className="actions">
        <button type="button" onClick={onCancel}>
          {t("skills.cancel")}
        </button>
        <button type="submit" className="primary" disabled={busy || !loaded || !canSave(draft)}>
          {busy ? t(skill ? "skills.saving" : "skills.creating") : t(skill ? "skills.save" : "skills.create")}
        </button>
      </div>
      {skill && (
        <section className="danger-zone">
          <h2>{t("skills.deleteTitle")}</h2>
          <p className="muted">{inUse ? t("skills.inUse", { bots: inUse }) : t("skills.deleteHint")}</p>
          <div>
            <button type="button" className="danger" disabled={busy || inUse !== ""} onClick={() => void remove()}>
              {t("skills.delete")}
            </button>
          </div>
        </section>
      )}
    </form>
  );
}
