import { useTranslation } from "react-i18next";
import type { ProviderStatus } from "../api";
import { providerState } from "../setup";

interface Props {
  providers: ProviderStatus[] | null;
  onCheck: () => void;
}

export default function ProviderChecks({ providers, onCheck }: Props) {
  const { t } = useTranslation();
  return (
    <div className="checks">
      {providers === null ? (
        <p className="muted">{t("tools.checking")}</p>
      ) : (
        <ul>
          {providers.map((p) => {
            const state = providerState(p);
            return (
              <li key={p.provider} className={`check-row ${state}`}>
                <strong>{t(`providers.${p.provider}`)}</strong>
                <span>{t(`tools.${state}`)}</span>
                {p.version && <span className="muted">{p.version}</span>}
                {(state === "missing" || state === "notLoggedIn") && (
                  <span className="muted">{t(`tools.fix.${p.provider}.${state}`)}</span>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {providers !== null && !providers.some((p) => p.path !== null) && (
        <p className="error-text">{t("tools.noneFound")}</p>
      )}
      <div>
        <button onClick={onCheck} disabled={providers === null}>
          {t("tools.checkAgain")}
        </button>
      </div>
    </div>
  );
}
