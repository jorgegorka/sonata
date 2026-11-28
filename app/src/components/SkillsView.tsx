import { PageHeader, StatusBadge } from "./ui";
import { useProjectApi } from "../projectContext";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { type Skill, type SkillView } from "../api";
import { usedByKey } from "../skills";

interface Props {
  skills: SkillView[];
  onNew: () => void;
  onOpen: (skillId: string) => void;
  onImported: (skill: Skill) => void;
  onError: (error: unknown) => void;
}

export default function SkillsView({ skills, onNew, onOpen, onImported, onError }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();

  async function importFolder() {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked !== "string") return;
      onImported(await api.importSkill(picked));
    } catch (e) {
      onError(e);
    }
  }

  const bots = (handles: string[]) => handles.map((h) => `@${h}`).join(", ");
  return (
    <>
      <PageHeader title={t("skills.title")} description={t("skills.intro")} actions={<>
        <button className="primary" onClick={onNew}>{t("skills.newSkill")}</button>
        <button onClick={() => void importFolder()}>{t("skills.import")}</button>
        <button onClick={() => void api.openSkillsFolder().catch(onError)}>{t("skills.showFolder")}</button>
      </>} />
      {skills.length === 0 && <p className="empty-state">{t("skills.empty")}</p>}
      <ul className="resource-list">
        {skills.map((s) => (
          <li key={s.id}>
            <button className="resource-row" onClick={() => onOpen(s.id)}>
              <span className="resource-heading">
                <strong>{s.name}</strong>
                <StatusBadge>{t(`skills.${s.source}`)}</StatusBadge>
              </span>
              <span className="muted">{s.description}</span>
              <span className="muted">
                {t(usedByKey(s.used_by), { bots: bots(s.used_by), count: s.used_by.length })}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}
