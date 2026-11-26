// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useUpdates, type UpdateState } from "./updates";

const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Set<(event: { payload: UpdateState }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: UpdateState }) => void) => {
    bridge.listeners.add(handler); return () => bridge.listeners.delete(handler);
  },
}));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLElement, root: Root;
const idle: UpdateState = { revision: 0, phase: "idle", version: null, installation: null, downloaded: 0, total: null };
function View({ loaded }: { loaded: boolean }) {
  const state = useUpdates(loaded);
  return <output>{state?.phase ?? "waiting"}</output>;
}
const render = (loaded = true) => act(async () => root.render(<StrictMode><View loaded={loaded} /></StrictMode>));
beforeEach(() => {
  bridge.listeners.clear(); bridge.invoke.mockReset().mockResolvedValue(idle);
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("waits for settings and keeps one listener through StrictMode", async () => {
  await render(false); expect(bridge.invoke).not.toHaveBeenCalled();
  await render();
  expect(bridge.listeners.size).toBe(1);
  expect(bridge.invoke.mock.calls.filter(([name]) => name === "check_startup_update")).toHaveLength(1);
  await render(); expect(bridge.invoke.mock.calls.filter(([name]) => name === "check_startup_update")).toHaveLength(1);
});

it("does not replace a newer event with an older startup response", async () => {
  let finish!: (state: UpdateState) => void;
  bridge.invoke.mockImplementation((name: string) => name === "check_startup_update"
    ? new Promise<UpdateState>(resolve => { finish = resolve; }) : Promise.resolve(idle));
  await render();
  await act(async () => bridge.listeners.forEach(handler => handler({ payload: { ...idle, revision: 3, phase: "dismissed" } })));
  await act(async () => finish({ ...idle, revision: 2, phase: "available", version: "1.2.0" }));
  expect(host.textContent).toBe("dismissed");
});

it("remounting observes the backend's dismissed state", async () => {
  bridge.invoke.mockResolvedValue({ ...idle, revision: 5, phase: "dismissed" });
  await render(); expect(host.textContent).toBe("dismissed");
  await act(async () => root.render(null));
  expect(bridge.listeners.size).toBe(0);
  await render(); expect(host.textContent).toBe("dismissed");
});
