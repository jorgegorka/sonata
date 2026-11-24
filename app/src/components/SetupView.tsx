import { PROVIDERS } from "../botForm";
import { useProjectApi } from "../projectContext";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { type AppSettings, type Provider, type ProviderStatus, type ProjectInput } from "../api";
import HerdrCheck from "./HerdrCheck";
import ProviderChecks from "./ProviderChecks";

interface Props {
  providers: ProviderStatus[] | null;
  onCheckProviders: () => void;
  onError: (error: unknown) => void;
  onDone: () => void;
  initialProject?: ProjectInput | null;
  onProjectChange?: (project: ProjectInput) => void;
  embedded?: boolean;
}

export default function SetupView({ providers, onCheckProviders, onError, onDone, initialProject, onProjectChange, embedded }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [checkout, setCheckout] = useState<string | null>(initialProject?.default_directory ?? null);
  const [name, setName] = useState(initialProject?.name ?? "Default");
  const [provider, setProvider] = useState<Provider>(initialProject?.default_provider ?? providers?.find(p => p.path && p.logged_in !== false)?.provider ?? "claude");
  const [busy, setBusy] = useState(false);
  useEffect(() => { onProjectChange?.({ name, default_directory: checkout, default_provider: provider }); }, [name, checkout, provider, onProjectChange]);

  useEffect(() => {
    api.getSettings().then(setSettings).catch(onError);
  }, [onError]);

  async function chooseFolder() {
    try {
      const picked = await open({ directory: true, multiple: false, defaultPath: settings?.bots_root });
      if (typeof picked !== "string") return;
      await api.setBotsRoot(picked);
      setSettings(await api.getSettings());
    } catch (e) {
      onError(e);
    }
  }

  async function chooseCheckout() {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string") setCheckout(picked);
    } catch (e) {
      onError(e);
    }
  }

  async function finish() {
    setBusy(true);
    try {
      await api.finishProjectSetup({ name, default_directory: checkout, default_provider: provider });
      onDone();
    } catch (e) {
      onError(e);
      setBusy(false);
      api.getSettings().then(setSettings).catch(onError);
    }
  }

  return (
    <div className={embedded ? "setup embedded" : "setup"}>
      {!embedded && <><h1>{t("setup.title")}</h1><p>{t("setup.intro")}</p></>}
      <section>
        <h2>{t("setup.safetyTitle")}</h2>
        <p>{t("setup.safety")}</p>
      </section>
      <details>
        <summary>{t("onboarding.advanced")}</summary>
        <h2>{t("setup.folderTitle")}</h2>
        <p className="muted">{t("setup.folderHint")}</p>
        <p className="path">{settings?.bots_root}</p>
        {settings &&
          (settings.bots_exist ? (
            <p className="muted">{t("setup.folderLocked")}</p>
          ) : (
            <div>
              <button disabled={busy} onClick={() => void chooseFolder()}>{t("setup.changeFolder")}</button>
            </div>
          ))}
      </details>
      <section>
        <h2>{t("setup.projectTitle")}</h2>
        <label>{t("projects.name")}<input disabled={busy} value={name} onChange={(e) => setName(e.target.value)} required /></label>
        <label>{t("projects.provider")}<select disabled={busy} value={provider} onChange={(e) => setProvider(e.target.value as Provider)}>
          {PROVIDERS.map((p) => <option key={p} value={p}>{t(`providers.${p}`)}</option>)}
        </select></label>
        <p className="muted">{t(embedded ? "onboarding.folderHint" : "setup.projectHint")}</p>
        <p className="path">{checkout ?? t("setup.projectNone")}</p>
        <div>
          <button disabled={busy} onClick={() => void chooseCheckout()}>{t("setup.chooseCheckout")}</button>
        </div>
      </section>
      <section>
        <h2>{t("setup.toolsTitle")}</h2>
        <p className="muted">{t("setup.toolsHint")}</p>
        <ProviderChecks providers={providers} onCheck={onCheckProviders} />
        <HerdrCheck onError={onError} />
      </section>
      <div className="actions">
        <button className="primary" disabled={busy || !name.trim() || !checkout} onClick={() => void finish()}>
          {t(embedded ? "onboarding.next" : "setup.finish")}
        </button>
      </div>
    </div>
  );
}
