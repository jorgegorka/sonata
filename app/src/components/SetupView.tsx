import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type AppSettings, type ProviderStatus } from "../api";
import HerdrCheck from "./HerdrCheck";
import ProviderChecks from "./ProviderChecks";

interface Props {
  providers: ProviderStatus[] | null;
  onCheckProviders: () => void;
  onError: (error: unknown) => void;
  onDone: () => void;
}

export default function SetupView({ providers, onCheckProviders, onError, onDone }: Props) {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [checkout, setCheckout] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
      await api.finishSetup(checkout);
      onDone();
    } catch (e) {
      onError(e);
      setBusy(false);
      api.getSettings().then(setSettings).catch(onError);
    }
  }

  return (
    <div className="setup">
      <h1>{t("setup.title")}</h1>
      <p>{t("setup.intro")}</p>
      <section>
        <h2>{t("setup.safetyTitle")}</h2>
        <p>{t("setup.safety")}</p>
      </section>
      <section>
        <h2>{t("setup.folderTitle")}</h2>
        <p className="muted">{t("setup.folderHint")}</p>
        <p className="path">{settings?.bots_root}</p>
        {settings &&
          (settings.bots_exist ? (
            <p className="muted">{t("setup.folderLocked")}</p>
          ) : (
            <div>
              <button onClick={() => void chooseFolder()}>{t("setup.changeFolder")}</button>
            </div>
          ))}
      </section>
      <section>
        <h2>{t("setup.projectTitle")}</h2>
        <p className="muted">{t("setup.projectHint")}</p>
        <p className="path">{checkout ?? t("setup.projectNone")}</p>
        <div>
          <button onClick={() => void chooseCheckout()}>{t("setup.chooseCheckout")}</button>
        </div>
      </section>
      <section>
        <h2>{t("setup.toolsTitle")}</h2>
        <p className="muted">{t("setup.toolsHint")}</p>
        <ProviderChecks providers={providers} onCheck={onCheckProviders} />
        <HerdrCheck onError={onError} />
      </section>
      <div className="actions">
        <button className="primary" disabled={busy} onClick={() => void finish()}>
          {t("setup.finish")}
        </button>
      </div>
    </div>
  );
}
