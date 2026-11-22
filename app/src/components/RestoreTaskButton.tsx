import { useTranslation } from "react-i18next";
import { api } from "../api";
import { useAct } from "./useAct";

export default function RestoreTaskButton({ taskId, onError }: { taskId: string; onError: (e: unknown) => void }) {
  const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  return (
    <button disabled={busy} onClick={() => act(() => api.restoreSession(taskId))}>
      {t("archive.restore")}
    </button>
  );
}
