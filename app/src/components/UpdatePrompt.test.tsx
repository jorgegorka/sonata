// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeAll, beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import i18n from "../i18n";
import UpdatePrompt from "./UpdatePrompt";
import type { UpdateState } from "../updates";

const api = vi.hoisted(() => ({
  download: vi.fn(), install: vi.fn(), dismiss: vi.fn(), restart: vi.fn(), openLink: vi.fn(),
}));
vi.mock("../updates", async original => ({ ...await original<typeof import("../updates")>(), updatesApi: api }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLElement;
const error = vi.fn();
let state: UpdateState;
const button = (name: string) => [...host.querySelectorAll("button")].find(b => b.textContent === name);
const click = (name: string) => act(async () => button(name)!.click());
const render = () => act(async () => root.render(<UpdatePrompt state={state} onError={error} />));

beforeAll(async () => {
  await i18n.changeLanguage("en");
  HTMLDialogElement.prototype.showModal = function () { this.open = true; };
  HTMLDialogElement.prototype.close = function () { this.open = false; };
});
beforeEach(() => {
  Object.values(api).forEach(mock => mock.mockReset().mockResolvedValue(undefined)); error.mockReset();
  state = { revision: 1, phase: "available", version: "1.2.0", installation: "macos", downloaded: 0, total: null };
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

describe("update confirmations", () => {
  it("requires a download click and a separate restart click", async () => {
    await render();
    expect(host.querySelector("dialog")?.open).toBe(true);
    expect(api.download).not.toHaveBeenCalled(); expect(api.install).not.toHaveBeenCalled();
    await click("Download update");
    expect(api.download).toHaveBeenCalledOnce(); expect(api.install).not.toHaveBeenCalled();
    state = { ...state, phase: "downloading", downloaded: 10, total: 100 }; await render();
    expect(host.querySelector("progress")?.value).toBe(10);
    expect(button("Later")).toBeUndefined();
    state = { ...state, phase: "ready" }; await render();
    expect(host.textContent).toContain("all projects");
    expect(api.install).not.toHaveBeenCalled();
    await click("Restart and install"); expect(api.install).toHaveBeenCalledOnce();
  });

  it("Later and Escape dismiss without downloading or installing", async () => {
    await render(); await click("Later");
    expect(api.dismiss).toHaveBeenCalledOnce();
    await act(async () => host.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
    expect(api.dismiss).toHaveBeenCalledTimes(2);
    expect(api.download).not.toHaveBeenCalled(); expect(api.install).not.toHaveBeenCalled();
    state = { ...state, phase: "dismissed" }; await render();
    expect(host.querySelector("dialog")).toBeNull();
  });

  it("blocks duplicate download clicks while pending", async () => {
    let finish!: () => void;
    api.download.mockReturnValue(new Promise<void>(resolve => { finish = resolve; }));
    await render(); await click("Download update"); await click("Download update");
    expect(api.download).toHaveBeenCalledOnce();
    await act(async () => finish());
  });

  it("offers download retry without a restart after verification fails", async () => {
    state.phase = "download_failed"; await render();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("verified");
    await click("Try again");
    expect(api.download).toHaveBeenCalledOnce(); expect(api.install).not.toHaveBeenCalled();
  });

  it("requires restarting after an installation failure and cannot dismiss", async () => {
    state.phase = "install_failed"; await render();
    expect(button("Later")).toBeUndefined();
    await act(async () => host.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
    expect(api.dismiss).not.toHaveBeenCalled();
    await click("Restart Sonata"); expect(api.restart).toHaveBeenCalledOnce();
  });

  it.each(["arch", "extracted"] as const)("guides %s installs without invoking the native installer", async installation => {
    state.installation = installation; await render();
    await click("View update instructions");
    expect(host.textContent).toContain(installation === "arch" ? "pacman -U" : "--version 1.2.0");
    await click("Download update");
    expect(api.openLink).toHaveBeenCalledWith(true);
    expect(api.download).not.toHaveBeenCalled(); expect(api.install).not.toHaveBeenCalled();
  });

  it("hides quiet check and no-update results", async () => {
    for (const phase of ["checking", "idle", "dismissed"] as const) {
      state.phase = phase; await render(); expect(host.querySelector("dialog")).toBeNull();
    }
  });
});
