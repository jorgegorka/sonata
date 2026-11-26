import { useProjectApi } from "../projectContext";
import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { type AppSettings, type ImportPreview, type ProviderStatus } from "../api";
import { exportFileName } from "../configImport";
import i18n from "../i18n";
import { LANGUAGES, nativeName } from "../language";
import HerdrCheck from "./HerdrCheck";
import ImportPanel from "./ImportPanel";
import ProviderChecks from "./ProviderChecks";

interface Props {
  providers: ProviderStatus[] | null;
  onCheckProviders: () => void;
  onBotsChanged: () => void;
  onError: (error: unknown) => void;
  onPersonalize?: () => void;
}

export default function SettingsView({ providers, onCheckProviders, onBotsChanged, onError, onPersonalize }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [hopLimit, setHopLimit] = useState("");
  const [saved, setSaved] = useState(false);
  const [updatesSaving, setUpdatesSaving] = useState(false);
  const [checkoutKept, setCheckoutKept] = useState<string[] | null>(null);

  const [exported, setExported] = useState<string | null>(null);
  const [importing, setImporting] = useState<{ path: string; preview: ImportPreview } | null>(null);
  const [picks, setPicks] = useState(0);
  const [configBusy, setConfigBusy] = useState(false);

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setHopLimit(String(s.hop_limit));
      })
      .catch(onError);
  }, [onError]);

  async function saveHopLimit(e: FormEvent) {
    e.preventDefault();
    setSaved(false);
    const limit = Number(hopLimit);
    if (!Number.isInteger(limit)) {
      onError({ code: "hopLimitRange", params: { min: "1", max: "100" } });
      return;
    }
    try {
      await api.setHopLimit(limit);
      setSaved(true);
    } catch (err) {
      onError(err);
    }
  }

  async function chooseCheckout() {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        defaultPath: settings?.project_folder ?? undefined,
      });
      if (typeof picked !== "string") return;
      setCheckoutKept(null);
      const kept = await api.setProjectFolder(picked);
      setSettings((s) => s && { ...s, project_folder: picked });
      setCheckoutKept(kept);
      onBotsChanged();
    } catch (err) {
      onError(err);
    }
  }

  function configFilters() {
    return [{ name: t("config.fileFilter"), extensions: ["yaml", "yml"] }];
  }

  async function exportConfig() {
    try {
      const path = await save({ defaultPath: exportFileName(new Date()), filters: configFilters() });
      if (typeof path !== "string") return;
      setImporting(null);
      setConfigBusy(true);
      const summary = await api.exportConfig(path);
      const file = path.split("/").pop() ?? path;
      setExported(
        t("config.exported", {
          file,
          bots: summary.bots,
          skills: summary.skills,
          workflows: summary.workflows,
        }),
      );
    } catch (err) {
      onError(err);
    } finally {
      setConfigBusy(false);
    }
  }

  async function importConfig() {
    try {
      const path = await open({ multiple: false, directory: false, filters: configFilters() });
      if (typeof path !== "string") return;
      setConfigBusy(true);
      const preview = await api.previewConfigImport(path);
      setExported(null);
      setPicks((n) => n + 1);
      setImporting({ path, preview });
    } catch (err) {
      onError(err);
    } finally {
      setConfigBusy(false);
    }
  }

  async function changeLanguage(choice: string) {
    try {
      const resolved = await api.setLanguage(choice);
      await i18n.changeLanguage(resolved);
      setSettings((s) => s && { ...s, language: choice, language_resolved: resolved });
    } catch (err) {
      onError(err);
    }
  }

  async function changeAutomaticUpdates(enabled: boolean) {
    setUpdatesSaving(true);
    try {
      await api.setAutomaticUpdates(enabled);
      setSettings(s => s && { ...s, automatic_updates: enabled });
    } catch (error) { onError(error); }
    finally { setUpdatesSaving(false); }
  }

  if (!settings) return null;
  return (
    <>
      {onPersonalize && <section><h2>{t("onboarding.personalize")}</h2><p className="muted">{t("onboarding.returnHint")}</p><div><button onClick={onPersonalize}>{t("onboarding.personalize")}</button></div></section>}
      <section>
        <h2>{t("settings.languageTitle")}</h2>
        <label className="inline-label">
          {t("settings.language")}
          <select value={settings.language} onChange={(e) => void changeLanguage(e.target.value)}>
            <option value="system">
              {t("settings.systemLanguage", { name: nativeName(settings.language_system) })}
            </option>
            {LANGUAGES.map((code) => (
              <option key={code} value={code}>
                {nativeName(code)}
              </option>
            ))}
          </select>
        </label>
        <p className="muted">{t("settings.languageHint")}</p>
      </section>
      <section>
        <h2>{t("updates.title")}</h2>
        <label className="updates-checkbox">
          <input type="checkbox" checked={settings.automatic_updates} disabled={updatesSaving}
            aria-describedby="automatic-updates-hint"
            onChange={event => void changeAutomaticUpdates(event.target.checked)} />
          {t("updates.automatic")}
        </label>
        <p className="muted" id="automatic-updates-hint">{t("updates.hint")}</p>
      </section>
      <section>
        <h2>{t("settings.teamTitle")}</h2>
        <form className="inline-form" onSubmit={saveHopLimit}>
          <label>
            {t("settings.hopLimit")}
            <input
              type="number"
              min={1}
              max={100}
              value={hopLimit}
              onChange={(e) => {
                setHopLimit(e.target.value);
                setSaved(false);
              }}
            />
          </label>
          <button type="submit">{t("settings.save")}</button>
          {saved && <span className="muted">{t("settings.saved")}</span>}
        </form>
        <p className="muted">{t("settings.hopLimitHint")}</p>
      </section>
      <section>
        <h2>{t("setup.toolsTitle")}</h2>
        <ProviderChecks providers={providers} onCheck={onCheckProviders} />
        <HerdrCheck onError={onError} />
      </section>
      <section>
        <h2>{t("settings.foldersTitle")}</h2>
        <div className="folder-row">
          <strong>{t("settings.botsFolder")}</strong>
          <span className="path">{settings.bots_root}</span>
          <button onClick={() => void api.openBotsFolder().catch(onError)}>{t("settings.show")}</button>
        </div>
        <div className="folder-row">
          <strong>{t("settings.logsFolder")}</strong>
          <span className="path">{settings.logs_dir}</span>
          <button onClick={() => void api.openLogsFolder().catch(onError)}>{t("settings.show")}</button>
        </div>
        <div className="folder-row">
          <strong>{t("settings.projectFolder")}</strong>
          <span className="path">{settings.project_folder ?? t("setup.projectNone")}</span>
          <button onClick={() => void chooseCheckout()}>{t("setup.chooseCheckout")}</button>
          {settings.project_folder && (
            <button onClick={() => void api.openProjectFolder().catch(onError)}>{t("settings.show")}</button>
          )}
          {checkoutKept &&
            (checkoutKept.length === 0 ? (
              <span className="muted">{t("settings.saved")}</span>
            ) : (
              <span className="muted">
                {t("settings.projectKept", { bots: checkoutKept.map((handle) => `@${handle}`).join(", ") })}
              </span>
            ))}
        </div>
        <p className="muted">{t("settings.projectHint")}</p>
      </section>
      <section>
        <h2>{t("config.title")}</h2>
        <p className="muted">{t("projects.sharedSkills")}</p>
        <div className="folder-row">
          <button disabled={configBusy} onClick={() => void exportConfig()}>
            {t("projects.export")}
          </button>
          <button disabled={configBusy} onClick={() => void importConfig()}>
            {t("projects.import")}
          </button>
        </div>
        {importing ? (
          <ImportPanel
            key={picks}
            path={importing.path}
            preview={importing.preview}
            busy={configBusy}
            onBusy={setConfigBusy}
            onClose={() => setImporting(null)}
            onImported={onBotsChanged}
            onError={onError}
          />
        ) : exported ? (
          <p className="muted">{exported}</p>
        ) : (
          <p className="muted">{t("config.hint")}</p>
        )}
      </section>
    </>
  );
}
