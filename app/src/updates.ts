import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface UpdateState {
  revision: number;
  phase: "idle" | "checking" | "available" | "downloading" | "ready" | "installing" | "download_failed" | "install_failed" | "dismissed";
  version: string | null;
  installation: "macos" | "appimage" | "deb" | "arch" | "extracted" | null;
  downloaded: number;
  total: number | null;
}

export const updatesApi = {
  check: () => invoke<UpdateState>("check_startup_update"),
  state: () => invoke<UpdateState>("get_update_state"),
  download: () => invoke<UpdateState>("download_update"),
  install: () => invoke<UpdateState>("install_update"),
  dismiss: () => invoke<UpdateState>("dismiss_update"),
  restart: () => invoke<void>("restart_after_update_failure"),
  openLink: (download = false) => invoke<void>("open_update_link", { download }),
};

export function updateIsVisible(state: UpdateState | null): boolean {
  return !!state && !["idle", "checking", "dismissed"].includes(state.phase);
}

export function useUpdates(settingsLoaded: boolean) {
  const [state, setState] = useState<UpdateState | null>(null);
  useEffect(() => {
    if (!settingsLoaded) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    const accept = (next: UpdateState) => {
      if (!cancelled && next) setState(current => !current || next.revision >= current.revision ? next : current);
    };
    void (async () => {
      const stop = await listen<UpdateState>("update-state", event => accept(event.payload));
      if (cancelled) { stop(); return; }
      unlisten = stop;
      accept(await updatesApi.state());
      if (!cancelled) accept(await updatesApi.check());
    })().catch(error => console.warn("Startup update check unavailable", error));
    return () => { cancelled = true; unlisten?.(); };
  }, [settingsLoaded]);
  return state;
}
