import { useProjectApi } from "../projectContext";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Background,
  Controls,
  ReactFlow,
  type Connection,
  type EdgeChange,
  type FitViewOptions,
  type NodeChange,
  type ReactFlowInstance,
} from "@xyflow/react";
import {
  type BotView,
  type NewWorkflow,
  type Workflow,
  type WorkflowDef,
  type WorkflowPos,
  type WorkflowStep,
} from "../api";
import { errorMessage } from "../errors";
import {
  addStep,
  connect,
  LIMITS,
  movedTo,
  newStepId,
  removeArrow,
  removeStep,
  setOutcome,
  tidy,
  toFlow,
  workflowProblems,
} from "../workflows";
import {
  arrowEdge,
  edgeTypes,
  flowLabels,
  laidOut,
  nodeTypes,
  useNodeSizes,
  type ArrowEdge,
  type FlowNode,
} from "./WorkflowMap";

interface Props {
  workflow: Workflow | null;
  bots: BotView[];
  otherNames: string[];
  onSaved: (workflow: Workflow) => void;
  onCancel: () => void;
}

type Selected = { kind: "node" | "edge"; id: string } | null;

const START = "start";
const DONE = "done";
const STEP_W = 200;
const STEP_H = 64;
const END_W = 120;
const END_H = 48;
const GAP = 80;
const FIT: FitViewOptions<FlowNode> = { padding: 0.2, maxZoom: 1 };

const EMPTY: WorkflowDef = {
  version: 1,
  start: null,
  steps: [],
  start_pos: { x: (STEP_W - END_W) / 2, y: 0 },
  done_pos: { x: (STEP_W - END_W) / 2, y: END_H + GAP + STEP_H + GAP / 2 },
};

function arrowAt(edgeId: string): { from: string; index: number } | null {
  const cut = edgeId.lastIndexOf(":");
  const from = edgeId.slice(0, cut);
  return from === START ? null : { from, index: Number(edgeId.slice(cut + 1)) };
}

function placeNew(def: WorkflowDef, anchorId: string | null): { pos: WorkflowPos; done: WorkflowPos | null } {
  const anchor =
    def.steps.find((s) => s.id === anchorId && s.pos) ?? [...def.steps].reverse().find((s) => s.pos !== null);
  const start = def.start_pos;
  const pos = anchor?.pos
    ? { x: anchor.pos.x, y: anchor.pos.y + STEP_H + GAP }
    : start
      ? { x: start.x - (STEP_W - END_W) / 2, y: start.y + END_H + GAP }
      : { x: 0, y: 0 };
  const boxes = def.steps.flatMap((s) => (s.pos ? [{ ...s.pos, w: STEP_W, h: STEP_H }] : []));
  if (start) boxes.push({ ...start, w: END_W, h: END_H });
  const covers = (b: { x: number; y: number; w: number; h: number }) =>
    pos.x < b.x + b.w && b.x < pos.x + STEP_W && pos.y < b.y + b.h && b.y < pos.y + STEP_H;
  while (boxes.some(covers)) pos.x += STEP_W + GAP / 2;
  const done = def.done_pos;
  if (done && covers({ ...done, w: END_W, h: END_H })) return { pos, done: { x: done.x, y: pos.y + STEP_H + GAP } };
  return { pos, done };
}

function cleaned(draft: NewWorkflow): NewWorkflow {
  return {
    name: draft.name.trim(),
    when_to_use: draft.when_to_use.trim(),
    def: {
      ...draft.def,
      steps: draft.def.steps.map((s) => ({
        ...s,
        name: s.name.trim(),
        arrows: s.arrows.map((a) => ({ ...a, outcome: a.outcome?.trim() || null })),
      })),
    },
  };
}

function typing(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

export default function WorkflowEditor({ workflow, bots, otherNames, onSaved, onCancel }: Props) {
  const api = useProjectApi();
  const { t } = useTranslation();
  const [draft, setDraft] = useState<NewWorkflow>(() =>
    workflow
      ? { name: workflow.name, when_to_use: workflow.when_to_use, def: laidOut(workflow.def) }
      : { name: "", when_to_use: "", def: EMPTY },
  );
  const [selected, setSelected] = useState<Selected>(null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const flowRef = useRef<ReactFlowInstance<FlowNode, ArrowEdge> | null>(null);
  const [fitAgain, setFitAgain] = useState(0);
  const { note, measured } = useNodeSizes();
  const def = draft.def;
  const editDef = useCallback(
    (change: (def: WorkflowDef) => WorkflowDef) => setDraft((d) => ({ ...d, def: change(d.def) })),
    [],
  );

  const botIds = useMemo(() => new Set(bots.map((b) => b.id)), [bots]);
  const problems = useMemo(() => workflowProblems(draft, botIds, otherNames), [draft, botIds, otherNames]);
  const problemTexts = [...new Set(problems.map((p) => t(`errors.${p.code}`, p.params)))];

  const flow = useMemo(() => {
    const { nodes, edges } = toFlow(def, bots, problems);
    const noStart = problems.some((p) => p.code === "workflowNoStart");
    const bad = new Set(
      problems.flatMap((p) =>
        p.stepId !== undefined && p.arrowIndex !== undefined ? [`${p.stepId}:${p.arrowIndex}`] : [],
      ),
    );
    return {
      nodes: nodes.map(
        (n): FlowNode => ({
          ...n,
          selected: selected?.kind === "node" && selected.id === n.id,
          deletable: n.type === "step",
          className: n.data.problem || (n.id === START && noStart) ? "wf-problem" : undefined,
        }),
      ),
      edges: edges.map((e): ArrowEdge => {
        const at = arrowAt(e.id);
        return {
          ...arrowEdge(e, bad.has(e.id) ? "problem" : null),
          selected: selected?.kind === "edge" && selected.id === e.id,
          data: at
            ? {
                outcome: def.steps.find((s) => s.id === at.from)?.arrows[at.index]?.outcome ?? "",
                onOutcome: (text: string) => editDef((d) => setOutcome(d, at.from, at.index, text)),
                onSelect: () => setSelected({ kind: "edge", id: e.id }),
              }
            : {},
        };
      }),
    };
  }, [def, bots, problems, selected, editDef]);
  const nodes = useMemo(() => measured(flow.nodes), [measured, flow.nodes]);
  const labels = useMemo(() => flowLabels(t), [t]);

  const select = useCallback((kind: "node" | "edge", changes: (NodeChange<FlowNode> | EdgeChange<ArrowEdge>)[]) => {
    for (const c of changes) {
      if (c.type !== "select") continue;
      if (c.selected) setSelected({ kind, id: c.id });
      else setSelected((current) => (current?.kind === kind && current.id === c.id ? null : current));
    }
  }, []);
  const onNodesChange = useCallback(
    (changes: NodeChange<FlowNode>[]) => {
      note(changes);
      const moves = changes.flatMap((c) =>
        c.type === "position" && c.position
          ? [{ id: c.id, pos: { x: Math.round(c.position.x), y: Math.round(c.position.y) } }]
          : [],
      );
      if (moves.length > 0) editDef((d) => moves.reduce((acc, m) => movedTo(acc, m.id, m.pos), d));
      select("node", changes);
    },
    [note, editDef, select],
  );
  const onEdgesChange = useCallback((changes: EdgeChange<ArrowEdge>[]) => select("edge", changes), [select]);

  function onConnect({ source, target }: Connection) {
    if (source === DONE || target === START) return;
    const next = connect(def, source, target);
    editDef(() => next);
    const arrows = next.steps.find((s) => s.id === source)?.arrows ?? [];
    if (arrows.length > 1) setSelected({ kind: "edge", id: `${source}:${arrows.findIndex((a) => a.to === target)}` });
  }

  const removeSelection = useCallback(() => {
    const current = selectedRef.current;
    if (!current || (current.kind === "node" && (current.id === START || current.id === DONE))) return false;
    if (current.kind === "node") editDef((d) => removeStep(d, current.id));
    else {
      const at = arrowAt(current.id);
      editDef((d) => (at ? removeArrow(d, at.from, at.index) : { ...d, start: null }));
    }
    setSelected(null);
    return true;
  }, [editDef]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.key === "Delete" || e.key === "Backspace") && !typing(e.target) && removeSelection()) e.preventDefault();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [removeSelection]);

  function addBox() {
    const id = newStepId(def);
    const { pos, done } = placeNew(def, selected?.kind === "node" ? selected.id : null);
    let next = addStep({ ...def, done_pos: done }, "", pos);
    if (next.start === null) next = connect(next, START, id);
    editDef(() => next);
    setSelected({ kind: "node", id });
    setFitAgain((n) => n + 1);
  }

  function tidyUp() {
    editDef(tidy);
    setFitAgain((n) => n + 1);
  }
  useEffect(() => {
    if (fitAgain > 0) void flowRef.current?.fitView(FIT);
  }, [fitAgain]);

  async function save() {
    if (problems.length > 0) return;
    setBusy(true);
    setError(null);
    try {
      const input = cleaned(draft);
      onSaved(workflow ? await api.updateWorkflow(workflow.id, input) : await api.createWorkflow(input));
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
    }
  }

  const step = selected?.kind === "node" ? def.steps.find((s) => s.id === selected.id) : undefined;
  const editStep = (id: string, change: Partial<WorkflowStep>) =>
    editDef((d) => ({ ...d, steps: d.steps.map((s) => (s.id === id ? { ...s, ...change } : s)) }));
  const nodeName = (id: string) =>
    id === START
      ? t("workflowEditor.start")
      : id === DONE
        ? t("workflowEditor.done")
        : def.steps.find((s) => s.id === id)?.name.trim() || t("workflowEditor.unnamed");
  const arrow = selected?.kind === "edge" ? flow.edges.find((e) => e.id === selected.id) : undefined;

  return (
    <div className="wf-editor">
      <div className="wf-head">
        <h1>{workflow ? t("workflowEditor.editTitle", { name: workflow.name }) : t("workflowEditor.createTitle")}</h1>
        <div className="wf-fields">
          <label>
            {t("workflowEditor.name")}
            <input
              value={draft.name}
              onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
              placeholder={t("workflowEditor.namePlaceholder")}
              autoFocus={!workflow}
            />
          </label>
          <label className="wf-when">
            {t("workflowEditor.when")}
            <input
              value={draft.when_to_use}
              onChange={(e) => setDraft((d) => ({ ...d, when_to_use: e.target.value }))}
              placeholder={t("workflowEditor.whenPlaceholder")}
            />
            <span className="muted">{t("workflowEditor.whenHint")}</span>
          </label>
        </div>
        <div className="wf-toolbar">
          <button onClick={addBox} disabled={def.steps.length >= LIMITS.steps}>
            {t("workflowEditor.addStep")}
          </button>
          <button onClick={tidyUp}>{t("workflowEditor.tidy")}</button>
          <span className="wf-spacer" />
          <button onClick={onCancel}>{t("workflowEditor.cancel")}</button>
          <button className="primary" disabled={busy || problems.length > 0} onClick={() => void save()}>
            {busy ? t("workflowEditor.saving") : t("workflowEditor.save")}
          </button>
        </div>
        {(problemTexts.length > 0 || error) && (
          <ul className="wf-problems">
            {error && (
              <li className="error-text" role="alert">
                {error}
              </li>
            )}
            {problemTexts.map((text) => (
              <li key={text} className="error-text">
                {text}
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="wf-body">
        <div className="wf-flow wf-canvas">
          <ReactFlow
            nodes={nodes}
            edges={flow.edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onPaneClick={() => setSelected(null)}
            onInit={(instance) => (flowRef.current = instance)}
            deleteKeyCode={null}
            selectionKeyCode={null}
            multiSelectionKeyCode={null}
            panOnScroll
            zoomOnDoubleClick={false}
            fitView
            fitViewOptions={FIT}
            ariaLabelConfig={labels}
          >
            <Background gap={16} />
            <Controls showInteractive={false} />
          </ReactFlow>
        </div>
        <aside className="wf-panel">
          {step ? (
            <>
              <label>
                {t("workflowEditor.stepName")}
                <input value={step.name} onChange={(e) => editStep(step.id, { name: e.target.value })} />
              </label>
              <label>
                {t("workflowEditor.bot")}
                <select value={step.bot_id} onChange={(e) => editStep(step.id, { bot_id: e.target.value })}>
                  {!botIds.has(step.bot_id) && <option value={step.bot_id}>{t("workflowEditor.noBot")}</option>}
                  {bots.map((b) => (
                    <option key={b.id} value={b.id}>
                      {b.display_name} (@{b.handle})
                    </option>
                  ))}
                </select>
              </label>
              <label className="work-check"><input type="checkbox" checked={step.automatic_child_review ?? false} onChange={(e) => editStep(step.id, { automatic_child_review: e.target.checked })} />{t("work.automaticReview")}</label>
              <label className="wf-instructions">
                {t("workflowEditor.instructions")}
                <textarea
                  value={step.instructions}
                  onChange={(e) => editStep(step.id, { instructions: e.target.value })}
                  rows={12}
                />
                <span className="muted">{t("workflowEditor.instructionsHint")}</span>
              </label>
              <div className="actions start">
                <button className="danger" onClick={removeSelection}>
                  {t("workflowEditor.deleteStep")}
                </button>
              </div>
            </>
          ) : arrow ? (
            <>
              <h2>{t("workflowEditor.arrowTitle", { from: nodeName(arrow.source), to: nodeName(arrow.target) })}</h2>
              <p className="muted">
                {arrow.source === START ? t("workflowEditor.startArrowHint") : t("workflowEditor.arrowHint")}
              </p>
              <div className="actions start">
                <button className="danger" onClick={removeSelection}>
                  {t("workflowEditor.deleteArrow")}
                </button>
              </div>
            </>
          ) : (
            <p className="muted">{t("workflowEditor.panelHint")}</p>
          )}
        </aside>
      </div>
    </div>
  );
}
