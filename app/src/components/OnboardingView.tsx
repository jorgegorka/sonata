import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { OnboardingCreated, OnboardingPreview, OnboardingProgress, OnboardingState, ProjectInput, Provider, ProviderStatus, SessionView } from "../api";
import { useProjectApi } from "../projectContext";
import { errorMessage } from "../errors";
import SetupView from "./SetupView";
import ProviderChecks from "./ProviderChecks";
import AttentionCard from "./AttentionCard";

interface Props {
  firstRun: boolean;
  providers: ProviderStatus[] | null;
  onCheckProviders: () => void;
  onDone: (showWorkflows: boolean) => void;
}
const emptyProgress: OnboardingProgress = { profile: { job: "", tasks: "" }, stage: "profile", project: null, selected: [], skip: false };

export default function OnboardingView({ firstRun, providers, onCheckProviders, onDone }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [state, setState] = useState<OnboardingState | null>(null);
  const [progress, setProgress] = useState(emptyProgress);
  const progressRef = useRef(progress);
  const [setupReady, setSetupReady] = useState(false);
  const [provider, setProvider] = useState<Provider | null>(null);
  const [preview, setPreview] = useState<OnboardingPreview | null>(null);
  const [created, setCreated] = useState<OnboardingCreated[]>([]);
  const [session, setSession] = useState<SessionView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [refresh, setRefresh] = useState(0);
  const heading = useRef<HTMLHeadingElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const writes = useRef<Promise<unknown>>(Promise.resolve());
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);

  const updateProgress = useCallback((next: OnboardingProgress) => {
    progressRef.current = next;
    setProgress(next);
  }, []);
  const adopt = useCallback((next: OnboardingState) => {
    if (!mounted.current) return;
    setState(next);
    updateProgress(next.progress);
  }, [updateProgress]);
  const save = useCallback((next: OnboardingProgress) => {
    clearTimeout(timer.current);
    const request = writes.current.catch(() => {}).then(() => api.saveOnboarding(next));
    writes.current = request;
    return request;
  }, [api]);
  const change = useCallback((next: OnboardingProgress) => {
    updateProgress(next);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => { void save(next).catch(e => { if (mounted.current) setError(e); }); }, 350);
  }, [save, updateProgress]);
  useEffect(() => () => { clearTimeout(timer.current); }, []);

  useEffect(() => {
    let disposed = false;
    Promise.all([api.getOnboarding(), api.getSettings(), api.listProjects(), api.listBots()]).then(([saved, settings, projects, bots]) => {
      if (disposed) return;
      const project = projects.find(p => p.master_id && bots.some(b => b.id === p.master_id)) ?? projects[0];
      setSetupReady(settings.setup_done);
      setProvider(bots.find(b => b.id === project?.master_id)?.provider ?? project?.default_provider ?? null);
      let next = saved.progress;
      if (!firstRun && (saved.status === "complete" || next.stage === "ready")) next = {
        ...next, stage: saved.proposal && saved.status !== "complete" ? "suggestions" : "profile", skip: false,
      };
      if (!settings.setup_done && ["suggestions", "ready"].includes(next.stage)) next = { ...next, stage: "workspace" };
      adopt({ ...saved, progress: next });
    }).catch(setError);
    return () => { disposed = true; };
  }, [api, firstRun, adopt]);

  useEffect(() => { heading.current?.focus(); }, [progress.stage]);
  useEffect(() => {
    if (state?.status !== "generating") return;
    let disposed = false;
    let pollTimer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const [next, sessions] = await Promise.all([api.getOnboarding(), api.listSessions()]);
        if (disposed) return;
        setSession(sessions.find(s => s.id === next.session_id) ?? null);
        adopt(next);
      } catch (e) { if (!disposed) setError(e); }
      if (!disposed) pollTimer = setTimeout(() => void poll(), 1000);
    };
    void poll();
    return () => { disposed = true; clearTimeout(pollTimer); };
  }, [api, state?.status, adopt]);

  const selectedKey = JSON.stringify(progress.selected);
  useEffect(() => {
    setPreview(null);
    if (!["suggestions", "ready"].includes(progress.stage) || !state?.bundle || !["ready", "partial", "complete"].includes(state.status) || progress.selected.length === 0) return;
    let disposed = false;
    api.previewOnboarding(progress.selected).then(next => {
      if (!disposed) { setPreview(next); setCreated(next.created); }
    }).catch(e => { if (!disposed) setError(e); });
    return () => { disposed = true; };
  }, [api, state?.generation, state?.status, progress.stage, selectedKey, refresh]);

  const act = async (work: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); setError(null);
    try { await work(); } catch (e) { if (mounted.current) setError(e); }
    finally { if (mounted.current) setBusy(false); }
  };
  const nextFromProfile = () => act(async () => {
    const saved = await save({ ...progressRef.current, stage: "profile" });
    const next = { ...saved.progress, skip: false, stage: setupReady ? "suggestions" as const : "workspace" as const };
    adopt(await save(next));
  });
  const changeProject = useCallback((project: ProjectInput) => {
    if (JSON.stringify(progressRef.current.project) !== JSON.stringify(project)) change({ ...progressRef.current, project });
  }, [change]);
  const workspaceDone = () => act(async () => {
    setSetupReady(true);
    setProvider(progressRef.current.project?.default_provider ?? null);
    if (progressRef.current.skip) {
      await save(progressRef.current);
      await api.finishOnboarding();
      onDone(false);
    } else adopt(await save({ ...progressRef.current, stage: "suggestions" }));
  });
  const skip = () => act(async () => {
    if (state?.status === "generating") adopt(await api.cancelOnboarding());
    if (!setupReady) {
      adopt(await save({ ...progressRef.current, stage: "workspace", skip: true }));
    } else {
      await save({ ...progressRef.current, skip: true });
      await api.finishOnboarding();
      onDone(false);
    }
  });
  const generate = () => act(async () => {
    await save(progressRef.current);
    const health = await api.herdrStatus();
    if (health.error) throw { code: "onboardingRuntime", params: { error: health.error } };
    adopt(await api.generateOnboarding());
  });
  const edit = () => act(async () => {
    if (state?.status === "generating") adopt(await api.cancelOnboarding());
    adopt(await save({ ...progressRef.current, stage: "profile", skip: false }));
  });
  const create = () => act(async () => {
    if (!preview) return;
    await save(progressRef.current);
    const result = await api.applyOnboarding(progress.selected, preview.token);
    adopt(result.state); setCreated(result.created);
    setRefresh(n => n + 1);
    if (result.changed) setError({ code: "onboardingChanged" });
  });
  const finish = () => act(async () => { await api.finishOnboarding(); onDone(true); });
  const selectedProvider = providers?.find(p => p.provider === provider);
  const canGenerate = !!selectedProvider?.path && selectedProvider.logged_in !== false;
  const generating = state?.status === "generating";
  const stage = progress.stage;
  const config = state?.bundle;
  const displayError = error ?? state?.error;

  return <div className="onboarding">
    <p className="onboarding-eyebrow">{t("setup.title")}</p>
    <ol className="onboarding-steps" aria-label={t("onboarding.steps")}>
      {(["profile", "workspace", "suggestions", "ready"] as const).map((step, i) => <li key={step} aria-current={step === stage ? "step" : undefined}><span>{i + 1}</span>{t(`onboarding.step.${step}`)}</li>)}
    </ol>
    <h1 ref={heading} tabIndex={-1}>{t(`onboarding.title.${stage}`)}</h1>
    {displayError != null && <div className="error" role="alert">{errorMessage(t, displayError)}</div>}
    {!state && <p role="status">{t("onboarding.loading")}</p>}
    {state && stage === "profile" && <form onSubmit={e => { e.preventDefault(); void nextFromProfile(); }}>
      <p>{t("onboarding.intro")}</p>
      <label htmlFor="onboarding-job">{t("onboarding.job")}</label>
      <input id="onboarding-job" required disabled={busy} maxLength={200} value={progress.profile.job} placeholder={t("onboarding.jobExample")} onChange={e => change({ ...progress, profile: { ...progress.profile, job: e.target.value } })} />
      <label htmlFor="onboarding-tasks">{t("onboarding.tasks")}</label>
      <textarea id="onboarding-tasks" required disabled={busy} maxLength={8000} rows={5} value={progress.profile.tasks} placeholder={t("onboarding.tasksExample")} onChange={e => change({ ...progress, profile: { ...progress.profile, tasks: e.target.value } })} />
      <p className="muted">{t("onboarding.privacy")}</p>
      <div className="actions"><button type="button" disabled={busy} onClick={() => void skip()}>{t("onboarding.skip")}</button><button className="primary" disabled={busy || !progress.profile.job.trim() || !progress.profile.tasks.trim()}>{t("onboarding.next")}</button></div>
    </form>}
    {state && stage === "workspace" && <>
      <SetupView embedded initialProject={progress.project} onProjectChange={changeProject} providers={providers} onCheckProviders={onCheckProviders} onError={setError} onDone={() => void workspaceDone()} />
      <div className="actions"><button disabled={busy} onClick={() => void edit()}>{t("onboarding.back")}</button></div>
    </>}
    {state && stage === "suggestions" && <>
      <p>{t("onboarding.suggestionsIntro")}</p>
      {generating ? <>
        <p role="status" className="onboarding-progress">{t(session?.status === "limited" ? "status.limited" : "onboarding.generating")}</p>
        {session?.status === "needs_attention" && <AttentionCard session={session} onError={setError} />}
        <button disabled={busy} onClick={() => void act(async () => adopt(await api.cancelOnboarding()))}>{t("onboarding.cancel")}</button>
      </> : !state.proposal ? <>
        <ProviderChecks providers={providers} onCheck={onCheckProviders} />
        {provider && <p className="muted">{t("onboarding.provider", { provider: t(`providers.${provider}`) })}</p>}
        {!canGenerate && <p role="status">{t("onboarding.providerNeeded")}</p>}
        <button className="primary" disabled={busy || !canGenerate} onClick={() => void generate()}>{t(state.status === "idle" ? "onboarding.generate" : "onboarding.retry")}</button>
      </> : <>
        <div className="onboarding-cards">
          {state.proposal.workflows.map(workflow => {
            const usedBots = config?.bots.filter(b => workflow.definition.steps.some(s => s.bot === b.handle)) ?? [];
            const usedSkills = config?.skills.filter(s => usedBots.some(b => b.skills.includes(s.slug))) ?? [];
            return <article className="onboarding-card" key={workflow.name}>
              <label className="check"><input type="checkbox" checked={progress.selected.includes(workflow.name)} disabled={busy} onChange={e => {
                setError(null);
                change({ ...progress, selected: e.target.checked ? [...progress.selected, workflow.name] : progress.selected.filter(n => n !== workflow.name) });
              }} /><strong>{workflow.name}</strong></label>
              <p className="onboarding-overview">{workflow.when_to_use}</p><p className="onboarding-overview"><strong>{t("onboarding.output")}</strong> {workflow.expected_output}</p>
              <ol>{workflow.definition.steps.map(step => <li key={step.id}>{step.name} <span className="muted">@{step.bot}</span></li>)}</ol>
              <p className="muted">{t("onboarding.resources", { bots: usedBots.length, skills: usedSkills.length })}</p>
              {workflow.requirements.length > 0 && <p className="onboarding-requirements"><strong>{t("onboarding.requirements")}</strong> {workflow.requirements.join(" · ")}</p>}
              <details><summary>{t("onboarding.details")}</summary>
                <section><p>{workflow.when_to_use}</p><p>{workflow.expected_output}</p>{workflow.requirements.length > 0 && <><strong>{t("onboarding.requirements")}</strong><ul>{workflow.requirements.map((r,i) => <li key={i}>{r}</li>)}</ul></>}</section>
                {workflow.definition.steps.map(step => <section key={step.id}><h3>{step.name}</h3><p className="onboarding-instructions">{step.instructions}</p></section>)}
                {usedBots.map(bot => <section key={bot.handle}><h3>{bot.name} (@{bot.handle})</h3><p className="onboarding-instructions">{bot.instructions}</p></section>)}
                {usedSkills.map(skill => <section key={skill.slug}><h3>{skill.name}</h3>{skill.files.map(file => <div key={file.path}><strong>{file.path}</strong><pre>{file.text ?? t("onboarding.binaryFile")}</pre></div>)}</section>)}
              </details>
            </article>;
          })}
        </div>
        {state.status === "partial" && <><p role="status">{t("onboarding.partial", { count: created.length })}</p><ul>{created.map(item => <li key={item.id}>{item.name}</li>)}</ul></>}
        {preview && <p className="muted" role="status">{t("onboarding.selection", { workflows: preview.config.workflows.length, bots: preview.config.bots.length, skills: preview.config.skills.length })}</p>}
        <button className="primary" disabled={busy || !preview || progress.selected.length === 0} onClick={() => void create()}>{t(busy ? "onboarding.creating" : state.status === "partial" ? "onboarding.retryCreation" : "onboarding.create")}</button>
      </>}
      <div className="actions"><button disabled={busy} onClick={() => void edit()}>{t("onboarding.edit")}</button><button disabled={busy} onClick={() => void skip()}>{t("onboarding.skip")}</button></div>
    </>}
    {state && stage === "ready" && <>
      <p>{t("onboarding.ready")}</p>
      <ul>{progress.selected.map(name => <li key={name}>{name}</li>)}</ul>
      <p className="muted">{t("onboarding.manualRun")}</p>
      {preview && <p>{t("onboarding.resources", { bots: preview.config.bots.length, skills: preview.config.skills.length })}</p>}
      <button className="primary" disabled={busy} onClick={() => void finish()}>{t("onboarding.openWorkflows")}</button>
    </>}
  </div>;
}
