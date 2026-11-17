// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowImportItem, WorkflowImportPreview } from "../api";
import i18n from "../i18n";
import WorkflowImportPanel from "./WorkflowImportPanel";

const mocks = vi.hoisted(() => ({ preview: vi.fn(), apply: vi.fn() }));
vi.mock("../api", () => ({ api: { previewWorkflowImport: mocks.preview, importWorkflowConfig: mocks.apply } }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const callbacks = { onBusy: vi.fn(), onClose: vi.fn(), onImported: vi.fn() };
const item = (
  kind: WorkflowImportItem["kind"],
  name: string,
  extra: Partial<WorkflowImportItem> = {},
): WorkflowImportItem => ({
  kind,
  name,
  target: name,
  status: { kind: "new" },
  action: "add",
  can_keep: false,
  required_by: [],
  changed_fields: [],
  changed_files: [],
  used_by_bots: [],
  used_by_workflows: [],
  ...extra,
});
const preview = (items: WorkflowImportItem[], can_import = true, token = "one"): WorkflowImportPreview => ({
  items,
  can_import,
  token,
});
const rootItem = () => item("workflow", "Selected");
let root: Root;
let host: HTMLElement;
const button = (name: string) => [...host.querySelectorAll("button")].find((b) => b.textContent === name)!;
const click = (el: Element) => act(async () => void el.dispatchEvent(new MouseEvent("click", { bubbles: true })));
const change = (el: HTMLSelectElement | HTMLInputElement, value: string) =>
  act(async () => {
    const prototype = el.tagName === "SELECT" ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(el, value);
    el.dispatchEvent(new Event(el.tagName === "SELECT" ? "change" : "input", { bubbles: true }));
  });
async function mount(names = ["Selected"]) {
  await act(async () =>
    root.render(
      <WorkflowImportPanel
        path="/tmp/file.yaml"
        workflows={names.map((name) => ({ name, when_to_use: "", steps: 1 }))}
        {...callbacks}
      />,
    ),
  );
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe("workflow import preview", () => {
  beforeAll(() => i18n.changeLanguage("en"));
  beforeEach(() => {
    Object.values(callbacks).forEach((f) => f.mockReset());
    mocks.preview.mockReset().mockResolvedValue(preview([rootItem()]));
    mocks.apply.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("selects the only workflow automatically, but requires a selection in full exports", async () => {
    await mount(["Selected", "Other"]);
    expect(mocks.preview).not.toHaveBeenCalled();
    expect(button("Import workflow").disabled).toBe(true);
    await change(host.querySelector("select")!, "Selected");
    expect(mocks.preview).toHaveBeenCalledWith("/tmp/file.yaml", expect.objectContaining({ workflow: "Selected" }));
    expect(button("Import workflow").disabled).toBe(false);
  });

  it("requires individual decisions, recalculates dependencies, and submits the accepted token and choices", async () => {
    const bot = item("bot", "review", {
      status: { kind: "different" },
      action: "choose",
      can_keep: true,
      required_by: ["Review"],
      changed_fields: ["instructions"],
      used_by_workflows: ["Other workflow"],
    });
    mocks.preview.mockResolvedValueOnce(preview([rootItem(), bot, item("skill", "incoming")], false));
    await mount();
    expect(button("Import workflow").disabled).toBe(true);
    expect(host.textContent).toContain("Other workflow");
    expect(host.textContent).toContain("Instructions");
    mocks.preview.mockResolvedValueOnce(preview([rootItem(), { ...bot, action: "reuse" }], true, "resolved"));
    await change(host.querySelector('select[aria-label="Choice for @review"]')!, "keep_local");
    expect(host.textContent).not.toContain("incoming");
    mocks.apply.mockResolvedValue({
      outcome: "done",
      report: { written: [{ kind: "workflow", name: "Selected", replaced: false }], backup: null, error: null },
    });
    await click(button("Import workflow"));
    expect(mocks.apply).toHaveBeenCalledWith(
      "/tmp/file.yaml",
      expect.objectContaining({ bots: { review: "keep_local" } }),
      "resolved",
    );
    expect(callbacks.onImported).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain("Added: Selected");
  });

  it("keeps the name input mounted while updating previews and ignores out-of-order responses", async () => {
    const w = item("workflow", "Selected", { status: { kind: "unchanged" }, action: "reuse", can_keep: true });
    mocks.preview.mockResolvedValue(preview([w]));
    await mount();
    const first = deferred<WorkflowImportPreview>();
    mocks.preview.mockReturnValueOnce(first.promise);
    await change(host.querySelector('select[aria-label="Choice for Selected"]')!, "copy");
    const input = host.querySelector("input")!;
    expect(input).not.toBeNull();
    const second = deferred<WorkflowImportPreview>();
    mocks.preview.mockReturnValueOnce(second.promise);
    await change(input, "Copy name");
    expect(host.querySelector("input")).toBe(input);
    expect(button("Import workflow").disabled).toBe(true);
    await act(async () =>
      second.resolve(preview([{ ...w, target: "Copy name", status: { kind: "new" }, action: "add" }], true, "latest")),
    );
    await act(async () => first.resolve(preview([w], false, "stale")));
    expect(button("Import workflow").disabled).toBe(false);
    mocks.apply.mockResolvedValue({ outcome: "done", report: { written: [], backup: null, error: null } });
    await click(button("Import workflow"));
    expect(mocks.apply).toHaveBeenCalledWith(
      "/tmp/file.yaml",
      expect.objectContaining({ new_name: "Copy name" }),
      "latest",
    );
  });

  it("requires another review after a stale apply and reports partial writes and recovery", async () => {
    await mount();
    mocks.apply.mockResolvedValueOnce({ outcome: "changed", preview: preview([rootItem()], true, "new-token") });
    await click(button("Import workflow"));
    expect(callbacks.onImported).not.toHaveBeenCalled();
    expect(host.textContent).toContain(i18n.t("config.changed"));
    mocks.apply.mockResolvedValueOnce({
      outcome: "done",
      report: {
        written: [{ kind: "skill", name: "shared", replaced: true }],
        backup: { file_name: "before.yaml", kept_at: "/tmp/previous/before.yaml" },
        error: { code: "internal", params: { error: "Disk full" } },
      },
    });
    await click(button("Import workflow"));
    expect(mocks.apply.mock.calls[1][2]).toBe("new-token");
    expect(host.textContent).toContain("Replaced: shared");
    expect(host.textContent).toContain("/tmp/previous/before.yaml");
    expect(host.textContent).toContain("Disk full");
  });

  it("shows failures, permits cancellation, and never applies an invalid preview", async () => {
    mocks.preview.mockResolvedValue(
      preview(
        [
          rootItem(),
          item("skill", "missing", {
            action: "blocked",
            status: { kind: "invalid", code: "skillUnknown", params: { slug: "missing" } },
          }),
        ],
        false,
      ),
    );
    await mount();
    expect(host.textContent).toContain("Resolve every required item");
    expect(button("Import workflow").disabled).toBe(true);
    await click(button("Cancel"));
    expect(callbacks.onClose).toHaveBeenCalledTimes(1);
    expect(mocks.apply).not.toHaveBeenCalled();
  });

  it("shows replacement totals without using reserved translation options", async () => {
    mocks.preview.mockResolvedValue(
      preview([
        { ...rootItem(), action: "reuse", status: { kind: "unchanged" } },
        item("skill", "shared", { action: "replace", status: { kind: "different" }, can_keep: true }),
      ]),
    );
    await mount();
    expect(host.textContent).toContain("Add: 0 · Replace: 1 · Reuse: 1");
  });

  it("lets a corrupt local skill be replaced even when it cannot be kept", async () => {
    mocks.preview.mockResolvedValue(
      preview(
        [rootItem(), item("skill", "broken", { action: "choose", status: { kind: "different" }, can_keep: false })],
        false,
      ),
    );
    await mount();
    const select = host.querySelector('select[aria-label="Choice for broken"]')!;
    expect([...select.querySelectorAll("option")].map((o) => o.value)).toEqual(["", "replace"]);
  });
});
