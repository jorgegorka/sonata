import type { ImportMode, ImportPreview, ImportReport, PlannedItem } from "./api";

export function exportFileName(now: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `sonata-config-${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}.yaml`;
}

export type ConfigKind = "bots" | "skills" | "workflows";

export function itemName(kind: ConfigKind, name: string): string {
  return kind === "bots" ? `@${name}` : name;
}

export interface PreviewGroup {
  kind: ConfigKind;
  added: PlannedItem[];
  here: PlannedItem[];
  cant: PlannedItem[];
}

export function groupPreview(p: ImportPreview): PreviewGroup[] {
  const kinds = [
    ["bots", p.bots],
    ["skills", p.skills],
    ["workflows", p.workflows],
  ] as const;
  return kinds
    .filter(([, items]) => items.length > 0)
    .map(([kind, items]) => ({
      kind,
      added: items.filter((i) => i.status.kind === "new"),
      here: items.filter((i) => i.status.kind === "unchanged" || i.status.kind === "different"),
      cant: items.filter((i) => i.status.kind === "invalid"),
    }));
}

export function writes(p: ImportPreview, mode: ImportMode): number {
  const wanted = mode === "add_and_replace" ? ["new", "different"] : ["new"];
  return [...p.bots, ...p.skills, ...p.workflows].filter((i) => wanted.includes(i.status.kind)).length;
}

type WrittenItem = ImportReport["written"][number];

export function groupWritten(written: WrittenItem[]): { kind: ConfigKind; items: WrittenItem[] }[] {
  const kinds = [
    ["bots", "bot"],
    ["skills", "skill"],
    ["workflows", "workflow"],
  ] as const;
  return kinds
    .map(([kind, one]) => ({ kind, items: written.filter((w) => w.kind === one) }))
    .filter((group) => group.items.length > 0);
}

export function workflowExportFileName(name: string, now: Date): string {
  const safe =
    name
      .normalize("NFC")
      .replace(/[\/\\:\x00-\x1f\x7f]/g, "-")
      .trim()
      .replace(/^\.+|\.+$/g, "")
      .slice(0, 80) || "workflow";
  return exportFileName(now).replace("sonata-config-", `sonata-workflow-${safe}-`);
}
