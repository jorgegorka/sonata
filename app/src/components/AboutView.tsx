/// <reference types="vite/client" />
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import appIcon from "../../app-icon.svg";

interface Props { onError: (error: unknown) => void; }

export default function AboutView({ onError }: Props) {
  const { t } = useTranslation();
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => { getVersion().then(setVersion).catch(onError); }, [onError]);
  return (
    <div className="about">
      <div className="logo" role="img" aria-label={t("about.logoAlt")}>
        <img src={appIcon} width={136} height={136} alt="" />
      </div>
      <h1>{t("app.title")}</h1>
      <p>{t("about.contributors")}</p>
      {version && <p className="muted">{t("about.version", { version })}</p>}
    </div>
  );
}
