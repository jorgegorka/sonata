import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";

interface Props { onError: (error: unknown) => void; }

export default function AboutView({ onError }: Props) {
  const { t } = useTranslation();
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => { getVersion().then(setVersion).catch(onError); }, [onError]);
  return (
    <div className="about">
      <div className="logo" role="img" aria-label={t("about.logoAlt")}>
        <svg width="112" height="112" viewBox="0 0 1024 1024" aria-hidden="true">
          <rect width="1024" height="1024" rx="224" fill="#4f46e5" />
          <path d="M704 304C624 224 352 216 320 368C284 532 728 464 704 648C684 808 400 824 304 720"
            fill="none" stroke="white" strokeWidth="88" strokeLinecap="round" />
        </svg>
      </div>
      <h1>{t("app.title")}</h1>
      <p>{t("about.contributors")}</p>
      {version && <p className="muted">{t("about.version", { version })}</p>}
    </div>
  );
}
