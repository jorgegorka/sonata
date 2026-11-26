// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeAll, beforeEach, afterEach, expect, it, vi } from "vitest";
import i18n from "../i18n";
import SettingsView from "./SettingsView";

const api = vi.hoisted(() => ({ getSettings: vi.fn(), setAutomaticUpdates: vi.fn() }));
vi.mock("../api", () => ({ api }));
vi.mock("./HerdrCheck", () => ({ default: () => null }));
vi.mock("./ProviderChecks", () => ({ default: () => null }));
vi.mock("./ImportPanel", () => ({ default: () => null }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLElement;
const error = vi.fn();
const checkbox = () => host.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
const render = () => act(async () => root.render(<SettingsView providers={[]} onCheckProviders={vi.fn()} onBotsChanged={vi.fn()} onError={error} />));
beforeAll(() => i18n.changeLanguage("en"));
beforeEach(() => {
  api.getSettings.mockResolvedValue({ automatic_updates: true, hop_limit: 12, language: "en", language_system: "en" });
  api.setAutomaticUpdates.mockReset().mockResolvedValue(undefined); error.mockReset();
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("shows the persisted checkbox and saves immediately", async () => {
  await render();
  expect(checkbox().checked).toBe(true);
  expect(checkbox().closest("label")?.textContent).toContain("Automatic updates");
  await act(async () => checkbox().click());
  expect(api.setAutomaticUpdates).toHaveBeenCalledWith(false);
  expect(checkbox().checked).toBe(false);
});

it("retains the persisted value and reports a save failure", async () => {
  api.setAutomaticUpdates.mockRejectedValue(new Error("disk unavailable"));
  await render(); await act(async () => checkbox().click());
  expect(checkbox().checked).toBe(true);
  expect(error).toHaveBeenCalledOnce();
  expect(checkbox().disabled).toBe(false);
});

it("disables the checkbox while saving", async () => {
  let finish!: () => void;
  api.setAutomaticUpdates.mockReturnValue(new Promise<void>(resolve => { finish = resolve; }));
  await render(); await act(async () => checkbox().click());
  expect(checkbox().disabled).toBe(true);
  await act(async () => finish());
  expect(checkbox().checked).toBe(false);
});
