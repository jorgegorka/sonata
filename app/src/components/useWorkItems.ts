import { useEffect, useRef, useState } from "react";
import type { WorkItem } from "../api";
import { useProjectApi } from "../projectContext";

export function useWorkItems(onError: (e: unknown) => void): WorkItem[] | null {
  const api = useProjectApi();
  const [items, setItems] = useState<WorkItem[] | null>(null);
  const fail = useRef(onError); fail.current = onError;
  useEffect(() => {
    let disposed = false, generation = 0;
    let unlisten: (() => void) | undefined;
    const refresh = () => {
      const current = ++generation;
      api.listWorkItems().then((data) => {
        if (!disposed && current === generation && data) setItems(data);
      }).catch((e) => { if (!disposed) fail.current(e); });
    };
    setItems(null); refresh();
    api.onUiEvent((event) => {
      if (["work_items_changed", "orchestration_changed", "workflow_run_changed", "session_added", "session_updated", "session_removed", "session_status", "message_added", "message_updated"].includes(event.type)) refresh();
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch((e) => { if (!disposed) fail.current(e); });
    return () => { disposed = true; unlisten?.(); };
  }, [api]);
  return items;
}
