import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Glance, type SessionActivity, type SessionView } from "../api";
import { ACTIVITY_BATCH_MS, GLANCE_MS, activityTargets, glanceTargets, noteAttention } from "../board";
import { isArchived } from "../state";

export interface BoardData {
  activity: Record<string, SessionActivity>;
  glances: Record<string, Glance>;
  attentionSince: Record<string, string>;
}

function sameGlances(a: Record<string, Glance>, b: Record<string, Glance>): boolean {
  const ids = Object.keys(b);
  return (
    ids.length === Object.keys(a).length &&
    ids.every((id) => {
      const [x, y] = [a[id], b[id]];
      return x !== undefined && x.step === y.step && x.background_work === y.background_work && x.tail === y.tail;
    })
  );
}

export function useBoardData(sessions: SessionView[], onError: (e: unknown) => void): BoardData {
  const [activity, setActivity] = useState<Record<string, SessionActivity>>({});
  const [glances, setGlances] = useState<Record<string, Glance>>({});
  const [attentionSince, setAttentionSince] = useState<Record<string, string>>({});
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  const errorRef = useRef(onError);
  errorRef.current = onError;
  const reads = useRef({ count: 0, latest: new Map<string, number>() });

  const readActivity = useCallback((ids: string[]) => {
    if (ids.length === 0) return;
    const read = ++reads.current.count;
    for (const id of ids) reads.current.latest.set(id, read);
    api
      .sessionActivity(ids)
      .then((list) => {
        const newest = (id: string) => reads.current.latest.get(id) === read;
        const found = list.filter((a) => newest(a.session_id));
        const gone = ids.filter((id) => newest(id) && !list.some((a) => a.session_id === id));
        setActivity((prev) => {
          const next = { ...prev };
          for (const a of found) next[a.session_id] = a;
          for (const id of gone) delete next[id];
          return next;
        });
      })
      .catch((e: unknown) => errorRef.current(e));
  }, []);

  const known = useRef(new Set<string>());
  useEffect(() => {
    const fresh = sessions.filter((s) => !isArchived(s) && !known.current.has(s.id)).map((s) => s.id);
    for (const id of fresh) known.current.add(id);
    readActivity(fresh);
  }, [sessions, readActivity]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const pending = new Set<string>();
    api
      .onUiEvent((event) => {
        for (const id of activityTargets(event)) pending.add(id);
        if (pending.size === 0 || timer !== undefined) return;
        timer = setTimeout(() => {
          timer = undefined;
          const ids = [...pending];
          pending.clear();
          readActivity(ids);
        }, ACTIVITY_BATCH_MS);
      })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch((e: unknown) => errorRef.current(e));
    return () => {
      cancelled = true;
      unlisten?.();
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [readActivity]);

  useEffect(() => {
    let live = true;
    let busy = false;
    const tick = () => {
      if (busy || document.visibilityState === "hidden") return;
      const ids = glanceTargets(sessionsRef.current);
      if (ids.length === 0) {
        setGlances((prev) => (Object.keys(prev).length === 0 ? prev : {}));
        return;
      }
      busy = true;
      api
        .sessionGlances(ids)
        .then((list) => {
          if (!live) return;
          const next: Record<string, Glance> = Object.fromEntries(list.map((g) => [g.session_id, g]));
          setGlances((prev) => (sameGlances(prev, next) ? prev : next));
        })
        .catch((e: unknown) => console.warn("Could not glance at the sessions' screens", e))
        .finally(() => {
          busy = false;
        });
    };
    tick();
    const interval = setInterval(tick, GLANCE_MS);
    return () => {
      live = false;
      clearInterval(interval);
    };
  }, []);

  useEffect(() => setAttentionSince((prev) => noteAttention(prev, sessions, new Date().toISOString())), [sessions]);

  return { activity, glances, attentionSince };
}
