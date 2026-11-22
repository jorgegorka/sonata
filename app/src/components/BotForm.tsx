import { useProject } from "../projectContext";
import { useProjectApi } from "../projectContext";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { type Bot, type BotView, type Provider, type ProviderStatus, type SkillView } from "../api";
import { PROVIDERS, deletedAnyway, draftOf, editNotes, modelPlaceholderKey, type BotDraft } from "../botForm";
import { errorMessage } from "../errors";
import { availableProviders } from "../setup";
import { toggleId } from "../skills";

const COLORS = [
  "#2563eb",
  "#16a34a",
  "#dc2626",
  "#9333ea",
  "#ea580c",
  "#0891b2",
  "#db2777",
  "#ca8a04",
  "#0d9488",
  "#475569",
];

interface Props {
  bot?: BotView;
  providers: ProviderStatus[] | null;
  skills: SkillView[];
  taskCount?: number;
  onSaved: (bot: Bot) => void;
  onDeleted?: (botId: string) => void;
  onCancel?: () => void;
  onError: (error: unknown) => void;
}

export default function BotForm({ bot, providers, skills, taskCount = 0, onSaved, onDeleted, onCancel, onError }: Props) {
  const api = useProjectApi();
  const project = useProject();
  const [inheritProvider, setInheritProvider] = useState(!bot);
  const { t } = useTranslation();
  const available = availableProviders(providers, PROVIDERS);
  const [draft, setDraft] = useState<BotDraft>(() =>
    bot
      ? draftOf(bot)
      : {
          display_name: "",
          color: COLORS[0],
          provider: project?.default_provider ?? available[0] ?? PROVIDERS[0],
          model: "",
          instructions: t("createBot.instructionsTemplate"),
          skill_ids: [],
        },
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = <K extends keyof BotDraft>(key: K, value: BotDraft[K]) => setDraft((d) => ({ ...d, [key]: value }));
  const selectable = (p: Provider) => available.includes(p) || p === bot?.provider;
  const custom = !COLORS.includes(draft.color);

  useEffect(() => {
    if (bot || project) return;
    const found = availableProviders(providers, PROVIDERS);
    if (found.length > 0 && !found.includes(draft.provider)) setDraft((d) => ({ ...d, provider: found[0] }));
  }, [bot, providers, draft.provider]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const input = {
      display_name: draft.display_name,
      color: draft.color,
      provider: !bot && inheritProvider ? null : draft.provider,
      model: draft.model.trim() || null,
      instructions: draft.instructions,
      skill_ids: draft.skill_ids,
    };
    try {
      onSaved(bot ? await api.updateBot(bot.id, input) : await api.createBot(input));
    } catch (err) {
      setError(errorMessage(t, err));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!bot) return;
    const confirmed = await ask(t(taskCount > 0 ? "botForm.deleteConfirmTasks" : "botForm.deleteConfirm", {
        name: bot.display_name,
        count: taskCount,
      }), {
      title: t("botForm.deleteTitle"),
      kind: "warning",
      okLabel: t("botForm.delete"),
      cancelLabel: t("createBot.cancel"),
    });
    if (!confirmed) return;
    setBusy(true);
    try {
      await api.deleteBot(bot.id);
      onDeleted?.(bot.id);
    } catch (err) {
      if (deletedAnyway(err)) {
        onDeleted?.(bot.id);
        onError(err);
        return;
      }
      setError(errorMessage(t, err));
      setBusy(false);
    }
  }

  const notes = bot ? editNotes(bot, draft) : [];
  return (
    <form className="create-bot" onSubmit={submit}>
      <h1>{bot ? t("botForm.editTitle", { name: bot.display_name }) : t("createBot.title")}</h1>
      {bot && (
        <p className="muted">
          @{bot.handle} · {t("botForm.handleFixed")}
        </p>
      )}
      <label>
        {t("createBot.name")}
        <input
          value={draft.display_name}
          onChange={(e) => set("display_name", e.target.value)}
          placeholder={t("createBot.namePlaceholder")}
          required
          autoFocus
        />
      </label>
      <fieldset>
        <legend>{t("createBot.color")}</legend>
        <div className="swatches">
          {COLORS.map((c) => (
            <button
              type="button"
              key={c}
              className={c === draft.color ? "swatch selected" : "swatch"}
              style={{ background: c }}
              onClick={() => set("color", c)}
              aria-label={c}
              aria-pressed={c === draft.color}
            />
          ))}
          <span
            className={custom ? "swatch custom selected" : "swatch custom"}
            style={custom ? { background: draft.color } : undefined}
            title={t("createBot.customColor")}
          >
            <input
              type="color"
              value={draft.color}
              onChange={(e) => set("color", e.target.value)}
              aria-label={t("createBot.customColor")}
            />
          </span>
        </div>
      </fieldset>
      <label>
        {t("createBot.provider")}
        <select value={!bot && inheritProvider ? "default" : draft.provider} onChange={(e) => {
          setInheritProvider(e.target.value === "default");
          set("provider", e.target.value === "default" ? (project?.default_provider ?? draft.provider) : e.target.value as Provider);
        }}>
          {!bot && <option value="default">{t("projects.inherit", { provider: project?.default_provider ?? draft.provider })}</option>}
          {PROVIDERS.map((p) => (
            <option key={p} value={p} disabled={!selectable(p)}>
              {selectable(p) ? t(`providers.${p}`) : t("createBot.notInstalled", { name: t(`providers.${p}`) })}
            </option>
          ))}
        </select>
      </label>
      {!bot && available.length === 0 && <p className="muted">{t("createBot.noProviders")}</p>}
      <label>
        {t("createBot.model")}
        <input
          value={draft.model}
          onChange={(e) => set("model", e.target.value)}
          placeholder={t(modelPlaceholderKey(draft.provider))}
        />
      </label>
      <label>
        {t("createBot.instructions")}
        <textarea value={draft.instructions} onChange={(e) => set("instructions", e.target.value)} rows={10} />
      </label>
      <fieldset>
        <legend>{t("createBot.skills")}</legend>
        {skills.length === 0 && <p className="muted">{t("createBot.noSkills")}</p>}
        <ul className="checkbox-list">
          {skills.map((s) => (
            <li key={s.id}>
              <label className="check">
                <input
                  type="checkbox"
                  checked={draft.skill_ids.includes(s.id)}
                  onChange={() => set("skill_ids", toggleId(draft.skill_ids, s.id))}
                />
                <span>
                  {s.name} <span className="muted">{s.description}</span>
                </span>
              </label>
            </li>
          ))}
        </ul>
      </fieldset>
      {notes.length > 0 && (
        <ul className="notes">
          {notes.map((key) => (
            <li key={key} className="muted">
              {t(key)}
            </li>
          ))}
        </ul>
      )}
      {error && (
        <div className="error" role="alert">
          {error}
        </div>
      )}
      <div className="actions">
        {onCancel && (
          <button type="button" onClick={onCancel}>
            {t("createBot.cancel")}
          </button>
        )}
        <button
          type="submit"
          className="primary"
          disabled={busy || !draft.display_name.trim()}
        >
          {busy ? t(bot ? "botForm.saving" : "createBot.creating") : t(bot ? "botForm.save" : "createBot.create")}
        </button>
      </div>
      {bot && (
        <section className="danger-zone">
          <h2>{t("botForm.deleteTitle")}</h2>
          <p className="muted">{t("botForm.deleteHint")}</p>
          <div>
            <button type="button" className="danger" disabled={busy} onClick={() => void remove()}>
              {t("botForm.delete")}
            </button>
          </div>
        </section>
      )}
    </form>
  );
}
