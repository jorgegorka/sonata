import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

export type SettingsTab = "settings" | "skills";

interface Props {
  tab: SettingsTab;
  onTab: (tab: SettingsTab) => void;
  children: ReactNode;
}

export default function SettingsPage({ tab, onTab, children }: Props) {
  const { t } = useTranslation();
  const tabs: Array<[SettingsTab, string]> = [
    ["settings", t("settings.title")],
    ["skills", t("skills.title")],
  ];
  return (
    <div className="settings">
      <div className="tabs" role="tablist">
        {tabs.map(([id, label]) => (
          <button
            key={id}
            id={`settings-tab-${id}`}
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? "tab selected" : "tab"}
            onClick={() => onTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="tab-panel" role="tabpanel" aria-labelledby={`settings-tab-${tab}`}>
        {children}
      </div>
    </div>
  );
}
