import { useEffect, useRef, useState } from "react";
import type { Orchestration } from "../api";
import { useProjectApi } from "../projectContext";

export function useOrchestrations(onError: (e: unknown) => void): Orchestration[] {
  const api = useProjectApi();
  const [runs, setRuns] = useState<Orchestration[]>([]);
  const fail = useRef(onError);
  fail.current = onError;
  useEffect(() => {
    let disposed = false;
    let generation = 0;
    let unlisten: (() => void) | undefined;
    const refresh = () => {
      const current = ++generation;
      api.listOrchestrations().then((data) => {
        if (!disposed && current === generation) setRuns(data ?? []);
      }).catch((e) => { if (!disposed) fail.current(e); });
    };
    setRuns([]);
    refresh();
    api.onUiEvent((event) => {
      if (["orchestration_changed", "workflow_run_changed", "session_added", "session_removed", "session_updated"].includes(event.type)) refresh();
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch((e) => { if (!disposed) fail.current(e); });
    return () => { disposed = true; unlisten?.(); };
  }, [api]);
  return runs;
}
