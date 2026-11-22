import { useProjectApi } from "../projectContext";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { type BotView, type SessionView, type Workflow } from "../api";
import { EMPTY_DRAFT, type Draft } from "../drafts";
import Composer from "./Composer";
import WorkflowMap from "./WorkflowMap";

interface Props {
  workflow: Workflow;
  bots: BotView[];
  onStarted: (session: SessionView) => void;
  onError: (e: unknown) => void;
}

export default function WorkflowStart({ workflow, bots, onStarted, onError }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const { def } = workflow;
  const botless = def.steps.filter((s) => !bots.some((b) => b.id === s.bot_id));
  const first = bots.find((b) => b.id === def.steps.find((s) => s.id === def.start)?.bot_id);
  return (
    <div className="workflow-start">
      <div className="workflow-start-map">
        <WorkflowMap def={def} bots={bots} />
      </div>
      {botless.map((s) => (
        <p key={s.id} className="error-text drawer-note">
          {t("errors.workflowStepNoBot", { step: s.name })}
        </p>
      ))}
      {first && botless.length === 0 && (
        <Composer
          bot={first}
          sessionId={null}
          onSent={onStarted}
          autoFocus
          draft={draft}
          onDraftChange={setDraft}
          onError={onError}
          options={[]}
          send={(body, files) => api.startWorkflow(workflow.id, body, files)}
          placeholder={t("workflowStart.placeholder", { name: workflow.name })}
        />
      )}
    </div>
  );
}
