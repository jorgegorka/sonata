import { useEffect, useRef, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../errors";

/** Guard both async submission and dismissal, including completions after project changes. */
export function useFormGuard(dirty: boolean, onClose: () => void) {
  const { t } = useTranslation();
  const mounted = useRef(true);
  const active = useRef(false);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  async function save<T>(action: () => Promise<T>, onSaved: (value: T) => void) {
    if (active.current) return;
    active.current = true; setBusy(true); setError(null);
    try { const value = await action(); if (mounted.current) onSaved(value); }
    catch (e) { if (mounted.current) setError(errorMessage(t, e)); }
    finally { active.current = false; if (mounted.current) setBusy(false); }
  }
  async function requestClose() {
    if (active.current) return;
    if (!dirty) { onClose(); return; }
    active.current = true; setConfirming(true);
    try {
      if (await ask(t("editor.discardMessage"), { title: t("editor.discardTitle"), kind: "warning", okLabel: t("work.discard"), cancelLabel: t("work.keepEditing") }) && mounted.current) onClose();
    } catch (e) { if (mounted.current) setError(errorMessage(t, e)); }
    finally { active.current = false; if (mounted.current) setConfirming(false); }
  }
  return { busy, locked: busy || confirming, error, save, requestClose };
}
