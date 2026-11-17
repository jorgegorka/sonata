import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getSmoothStepPath,
  Handle,
  MarkerType,
  PanOnScrollMode,
  Position,
  ReactFlow,
  useStore,
  type AriaLabelConfig,
  type Edge,
  type EdgeProps,
  type EdgeTypes,
  type Node,
  type NodeChange,
  type NodeProps,
  type NodeTypes,
} from "@xyflow/react";
import type { BotView, WorkflowDef, WorkflowPos } from "../api";
import {
  BACK_SIDE,
  mapBounds,
  mapViewport,
  needsLayout,
  placeMissing,
  runMapState,
  stepCentre,
  tidy,
  toFlow,
  type FlowNodeData,
  type MapBox,
} from "../workflows";

export interface StepNodeData extends FlowNodeData {
  visits?: number;
  current?: boolean;
}
export type FlowNode = Node<StepNodeData>;

export interface ArrowData extends Record<string, unknown> {
  outcome?: string;
  onOutcome?: (text: string) => void;
  onSelect?: () => void;
}
export type ArrowEdge = Edge<ArrowData>;

export function laidOut(def: WorkflowDef): WorkflowDef {
  if (!needsLayout(def)) return def;
  return def.steps.every((s) => s.pos === null) ? tidy(def) : placeMissing(def);
}

export function arrowEdge(edge: Edge, mark: "problem" | "taken" | null): ArrowEdge {
  const color = mark === "problem" ? "var(--error)" : mark === "taken" ? "var(--accent)" : "var(--wf-edge)";
  return {
    ...edge,
    type: "outcome",
    className: mark === "problem" ? "wf-problem" : mark === "taken" ? "wf-taken" : undefined,
    markerEnd: { type: MarkerType.ArrowClosed, color, width: 16, height: 16 },
  };
}

export function flowLabels(t: TFunction): Partial<AriaLabelConfig> {
  return {
    "node.a11yDescription.default": t("workflowEditor.a11yNode"),
    "node.a11yDescription.keyboardDisabled": t("workflowEditor.a11yNode"),
    "node.a11yDescription.ariaLiveMessage": ({ x, y }) => t("workflowEditor.a11yMoved", { x, y }),
    "edge.a11yDescription.default": t("workflowEditor.a11yEdge"),
    "controls.ariaLabel": t("workflowEditor.controls"),
    "controls.zoomIn.ariaLabel": t("workflowEditor.zoomIn"),
    "controls.zoomOut.ariaLabel": t("workflowEditor.zoomOut"),
    "controls.fitView.ariaLabel": t("workflowEditor.fitView"),
    "handle.ariaLabel": t("workflowEditor.handle"),
  };
}

export function useNodeSizes() {
  const [sizes, setSizes] = useState<Record<string, { width: number; height: number }>>({});
  const note = useCallback((changes: NodeChange[]) => {
    setSizes((all) => {
      let next = all;
      for (const c of changes) {
        if (c.type !== "dimensions" || !c.dimensions) continue;
        const { width, height } = c.dimensions;
        if (all[c.id]?.width === width && all[c.id]?.height === height) continue;
        if (next === all) next = { ...all };
        next[c.id] = { width, height };
      }
      return next;
    });
  }, []);
  const measured = useCallback(
    <N extends Node>(nodes: N[]): N[] => nodes.map((n) => (sizes[n.id] ? { ...n, measured: sizes[n.id] } : n)),
    [sizes],
  );
  return { note, measured };
}

function StartNode() {
  const { t } = useTranslation();
  return (
    <div className="wf-node wf-end">
      {t("workflowEditor.start")}
      <Handle type="source" position={Position.Bottom} />
    </div>
  );
}

function DoneNode() {
  const { t } = useTranslation();
  return (
    <div className="wf-node wf-end">
      <Handle type="target" position={Position.Top} />
      {t("workflowEditor.done")}
    </div>
  );
}

function StepNode({ data }: NodeProps<FlowNode>) {
  const { t } = useTranslation();
  const color = data.color || undefined;
  return (
    <div className={data.current ? "wf-node wf-step wf-current" : "wf-node wf-step"} style={{ borderLeftColor: color }}>
      <Handle type="target" position={Position.Top} />
      <span className="wf-step-name">{data.name.trim() || t("workflowEditor.unnamed")}</span>
      <span className="wf-step-bot" style={{ color }}>
        {data.handle ? `@${data.handle}` : t("workflowEditor.noBot")}
      </span>
      {data.visits ? (
        <span className="wf-visits" title={t("workflowEditor.visits", { n: data.visits })}>
          {t("workflowEditor.visitBadge", { n: data.visits })}
        </span>
      ) : null}
      <Handle type="source" position={Position.Bottom} />
    </div>
  );
}

function OutcomeEdge({
  id,
  sourceX,
  sourceY,
  sourcePosition,
  targetX,
  targetY,
  targetPosition,
  markerEnd,
  label,
  selected,
  data,
}: EdgeProps<ArrowEdge>) {
  const { t } = useTranslation();
  const back = targetY < sourceY;
  const [path, labelX, labelY] = getSmoothStepPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
    borderRadius: 8,
    centerX: back ? Math.max(sourceX, targetX) + BACK_SIDE : undefined,
  });
  const onOutcome = selected ? data?.onOutcome : undefined;
  const keys = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur();
  };
  return (
    <>
      <BaseEdge id={id} path={path} markerEnd={markerEnd} interactionWidth={16} />
      {(onOutcome || label) && (
        <EdgeLabelRenderer>
          <div
            className={onOutcome ? "wf-edge-label editing nodrag nopan" : "wf-edge-label nodrag nopan"}
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
            onClick={data?.onSelect}
          >
            {onOutcome ? (
              <input
                value={data?.outcome ?? ""}
                onChange={(e) => onOutcome(e.target.value)}
                onKeyDown={keys}
                placeholder={t("workflowEditor.outcomePlaceholder")}
                aria-label={t("workflowEditor.outcome")}
                autoFocus
              />
            ) : (
              label
            )}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

function MapView({ bounds, focus }: { bounds: MapBox; focus: WorkflowPos | null }) {
  const width = useStore((s) => s.width);
  const height = useStore((s) => s.height);
  const panZoom = useStore((s) => s.panZoom);
  const fx = focus?.x;
  const fy = focus?.y;
  useEffect(() => {
    if (!panZoom || width <= 0 || height <= 0) return;
    const at = fx === undefined || fy === undefined ? null : { x: fx, y: fy };
    void panZoom.setViewport(mapViewport(bounds, at, { width, height }));
  }, [panZoom, width, height, bounds, fx, fy]);
  return null;
}

export const nodeTypes: NodeTypes = { start: StartNode, done: DoneNode, step: StepNode };
export const edgeTypes: EdgeTypes = { outcome: OutcomeEdge };

interface Props {
  def: WorkflowDef;
  bots: BotView[];
  state?: ReturnType<typeof runMapState>;
  onOpenStep?: (stepId: string) => void;
}

export default function WorkflowMap({ def, bots, state, onOpenStep }: Props) {
  const { t } = useTranslation();
  const { note, measured } = useNodeSizes();
  const laid = useMemo(() => laidOut(def), [def]);
  const bounds = useMemo(() => mapBounds(laid), [laid]);
  const current = state ? Object.keys(state.steps).find((id) => state.steps[id].current) : undefined;
  const focus = current === undefined ? null : stepCentre(laid, current);
  const flow = useMemo(() => {
    const { nodes, edges } = toFlow(laid, bots, []);
    return {
      nodes: nodes.map((n): FlowNode => {
        const step = state?.steps[n.id];
        return step ? { ...n, data: { ...n.data, visits: step.visits, current: step.current } } : n;
      }),
      edges: edges.map((e) => arrowEdge(e, state?.taken.has(e.id) ? "taken" : null)),
    };
  }, [laid, bots, state]);
  const nodes = useMemo(() => measured(flow.nodes), [measured, flow.nodes]);
  const labels = useMemo(() => flowLabels(t), [t]);
  return (
    <div className={onOpenStep ? "wf-flow wf-map opens" : "wf-flow wf-map"}>
      <ReactFlow
        nodes={nodes}
        edges={flow.edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={note}
        onNodeClick={onOpenStep ? (_, node) => node.type === "step" && onOpenStep(node.id) : undefined}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        panOnScroll
        panOnScrollMode={PanOnScrollMode.Vertical}
        zoomOnScroll={false}
        zoomOnDoubleClick={false}
        translateExtent={[
          [bounds.x, bounds.y],
          [bounds.x + bounds.width, bounds.y + bounds.height],
        ]}
        minZoom={0.1}
        ariaLabelConfig={labels}
      >
        <MapView bounds={bounds} focus={focus} />
      </ReactFlow>
    </div>
  );
}
