import { describe, expect, it } from "vitest";
import type { ImportPreview, ImportReport, PlannedItem } from "./api";
import { exportFileName, workflowExportFileName, groupPreview, groupWritten, itemName, writes } from "./configImport";

const item = (name: string, kind: string, target = name): PlannedItem => ({
  name,
  target,
  status: kind === "invalid" ? { kind: "invalid", code: "x", params: {} } : ({ kind } as PlannedItem["status"]),
});

describe("configImport", () => {
  it("names the export file after the local date", () =>
    expect(exportFileName(new Date(2026, 10, 3))).toBe("sonata-config-2026-11-03.yaml"));

  it("keeps workflow filenames in one directory and preserves readable names", () => {
    const date = new Date(2026, 10, 14);
    expect(workflowExportFileName("Review / test: rápido", date)).toBe(
      "sonata-workflow-Review - test- rápido-2026-11-14.yaml",
    );
    expect(workflowExportFileName("...", date)).toBe("sonata-workflow-workflow-2026-11-14.yaml");
  });

  it("groups the preview by kind, bots first, and leaves out empty kinds", () => {
    const bad = item("b2", "invalid");
    const p: ImportPreview = {
      token: "t",
      skills: [item("s1", "unchanged")],
      bots: [item("b1", "new"), bad],
      workflows: [],
    };
    expect(groupPreview(p)).toEqual([
      { kind: "bots", added: [item("b1", "new")], here: [], cant: [bad] },
      { kind: "skills", added: [], here: [item("s1", "unchanged")], cant: [] },
    ]);
  });

  it("counts what each mode would write", () => {
    const p: ImportPreview = {
      token: "t",
      skills: [item("a", "new"), item("b", "different"), item("c", "unchanged")],
      bots: [item("d", "new"), item("e", "unchanged"), item("f", "unchanged"), item("g", "invalid")],
      workflows: [],
    };
    expect(writes(p, "add_new")).toBe(2);
    expect(writes(p, "add_and_replace")).toBe(3);
  });

  it("writes a bot's handle with @, and a skill or workflow as it is", () => {
    expect(itemName("bots", "master-2")).toBe("@master-2");
    expect(itemName("skills", "sonata")).toBe("sonata");
    expect(itemName("workflows", "Ticket to merge request")).toBe("Ticket to merge request");
  });

  it("groups what was written by kind, bots first, and leaves out empty kinds", () => {
    const written: ImportReport["written"] = [
      { kind: "skill", name: "notes", replaced: false },
      { kind: "bot", name: "plan", replaced: true },
      { kind: "skill", name: "sonata", replaced: true },
      { kind: "bot", name: "writer", replaced: false },
    ];
    expect(groupWritten(written)).toEqual([
      { kind: "bots", items: [written[1], written[3]] },
      { kind: "skills", items: [written[0], written[2]] },
    ]);
    expect(groupWritten([])).toEqual([]);
  });
});
