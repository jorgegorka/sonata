import { useProjectApi } from "../projectContext";
import { useTranslation } from "react-i18next";
import { } from "../api";
import { useAct } from "./useAct";

export default function RestoreTaskButton({ taskId, workItemId, onError }: { workItemId?: string; taskId: string; onError: (e: unknown) => void }) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  return (
    <button disabled={busy} onClick={() => act(() => workItemId ? api.archiveWorkItem(workItemId, false) : api.restoreSession(taskId))}>
      {t("archive.restore")}
    </button>
  );
}
