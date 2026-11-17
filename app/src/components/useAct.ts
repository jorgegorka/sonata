import { useRef, useState } from "react";

export function useAct(onError: (e: unknown) => void) {
  const pending = useRef(false);
  const [busy, setBusy] = useState(false);
  const act = (call: () => Promise<unknown>) => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    call()
      .catch(onError)
      .finally(() => {
        pending.current = false;
        setBusy(false);
      });
  };
  return { busy, act };
}
