import { useCallback, useEffect, useRef, useState } from "react";
import type { WorkItem } from "../api";
import { useProjectApi } from "../projectContext";

const empty = { items: [] as WorkItem[], loaded: false, error: null as unknown };

export function useWorkItems() {
  const api = useProjectApi();
  const [state, setState] = useState({ ...empty, api });
  const request = useRef(() => {});
  const reload = useCallback(() => request.current(), []);
  useEffect(() => {
    let disposed = false, running = false, again = false;
    let unlisten: (() => void) | undefined;
    const refresh = async () => {
      if (running) { again = true; return; }
      running = true;
      do {
        again = false;
        try {
          const items = await api.listWorkItems();
          if (!Array.isArray(items)) throw new Error("Invalid task response");
          if (!disposed) setState({ api, items, loaded: true, error: null });
        } catch (error) {
          if (!disposed) setState((previous) => ({ ...previous, error }));
        }
      } while (again && !disposed);
      running = false;
    };
    request.current = () => { void refresh(); };
    setState({ ...empty, api });
    void refresh();
    api.onUiEvent((event) => {
      if (["work_items_changed", "orchestration_changed", "workflow_run_changed", "session_added", "session_updated", "session_removed", "session_status", "message_added", "message_updated"].includes(event.type)) void refresh();
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch((error) => {
      if (!disposed) setState((previous) => ({ ...previous, error }));
    });
    return () => { disposed = true; unlisten?.(); };
  }, [api]);
  const saved = (item: WorkItem) => {
    setState((previous) => previous.api !== api ? previous : {
      ...previous, items: [...previous.items.filter((w) => w.id !== item.id), item],
    });
    reload();
  };
  return { ...(state.api === api ? state : empty), reload, saved };
}
