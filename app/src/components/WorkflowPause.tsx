import { ask } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { api, type RunGlance } from "../api";
import type { Translate } from "../state";
import { LIMITS } from "../workflows";
import { useAct } from "./useAct";

interface Props {
  glance: RunGlance;
  onError: (e: unknown) => void;
}

async function stopConfirmed(glance: RunGlance, t: Translate): Promise<void> {
  const confirmed = await ask(t("board.workflowStopConfirm", { name: glance.name }), {
    title: t("board.workflowStop"),
    kind: "warning",
    okLabel: t("board.workflowStop"),
    cancelLabel: t("createBot.cancel"),
  });
  if (confirmed) await api.stopRun(glance.run_id);
}

export default function WorkflowPause({ glance, onError }: Props) {
  const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  const { pause, run_id: run, handle, outcomes } = glance;
  if (pause === null) return null;
  const step = glance.step ?? "";
  return (
    <div className="workflow-pause">
      <p className="task-what">{t(`board.workflowPaused_${pause.kind}`, { step, count: LIMITS.visitCap })}</p>
      <div className="task-actions">
        {pause.kind === "visit_cap" && (
          <button className="primary" disabled={busy} onClick={() => act(() => api.anotherRound(run))}>
            {t("board.workflowAnotherRound")}
          </button>
        )}
        {pause.kind === "no_result" &&
          (outcomes.length > 0 ? (
            outcomes.map((outcome) => (
              <button key={outcome} disabled={busy} onClick={() => act(() => api.pickOutcome(run, outcome))}>
                {outcome}
              </button>
            ))
          ) : (
            <button disabled={busy} onClick={() => act(() => api.pickOutcome(run, null))}>
              {t("board.workflowGoOn")}
            </button>
          ))}
        {pause.kind === "no_result" && handle !== null && (
          <button disabled={busy} onClick={() => act(() => api.remindStep(run))}>
            {t("board.workflowRemind", { handle })}
          </button>
        )}
        <button className="push" disabled={busy} onClick={() => act(() => stopConfirmed(glance, t))}>
          {t("board.workflowStop")}
        </button>
      </div>
    </div>
  );
}

export function StopRun({ glance, onError }: { glance: RunGlance; onError: (e: unknown) => void }) {
  const { t } = useTranslation();
  const { busy, act } = useAct(onError);
  return (
    <div className="task-actions">
      <button className="push" disabled={busy} onClick={() => act(() => stopConfirmed(glance, t))}>
        {t("board.workflowStop")}
      </button>
    </div>
  );
}
