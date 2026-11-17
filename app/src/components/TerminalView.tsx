import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { api } from "../api";
import {
  afterFailure,
  afterRead,
  INITIAL_TERMINAL,
  REFRESH_MS,
  showsReadFailed,
  TERMINAL_COLS,
  TERMINAL_ROWS,
  type TerminalState,
} from "../terminal";

interface Props {
  sessionId: string;
  onRunning?: (running: boolean) => void;
  noteWhenStopped?: boolean;
}

export default function TerminalView({ sessionId, onRunning, noteWhenStopped = true }: Props) {
  const { t } = useTranslation();
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const [state, setState] = useState<TerminalState>(INITIAL_TERMINAL);
  const running = useRef(onRunning);
  running.current = onRunning;

  useEffect(() => {
    if (!host.current) return;
    const terminal = new Terminal({
      cols: TERMINAL_COLS,
      rows: TERMINAL_ROWS,
      disableStdin: true,
      convertEol: true,
      cursorBlink: false,
      cursorInactiveStyle: "none",
      fontSize: 12,
    });
    terminal.open(host.current);
    term.current = terminal;
    return () => {
      terminal.dispose();
      term.current = null;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = () => {
      void api
        .sessionTerminal(sessionId)
        .then((result) => {
          if (cancelled) return;
          setState((s) => afterRead(s, result));
          running.current?.(result !== null);
        })
        .catch(() => {
          if (!cancelled) setState(afterFailure);
        })
        .finally(() => {
          if (!cancelled) timer = setTimeout(read, REFRESH_MS);
        });
    };
    read();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [sessionId]);

  useEffect(() => {
    const terminal = term.current;
    if (!terminal || state.screen === null) return;
    terminal.reset();
    terminal.write(state.screen, () => {
      if (host.current) host.current.scrollTop = host.current.scrollHeight;
    });
  }, [state.screen]);

  return (
    <div className="terminal-view">
      <div ref={host} className="terminal-host" hidden={state.running === false} />
      {noteWhenStopped && state.running === false && <p className="muted">{t("terminal.notRunning")}</p>}
      {showsReadFailed(state) && <p className="muted">{t("terminal.readFailed")}</p>}
    </div>
  );
}
