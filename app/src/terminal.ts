export const TERMINAL_COLS = 120;
export const TERMINAL_ROWS = 40;
export const REFRESH_MS = 2000;

export interface TerminalState {
  screen: string | null;
  running: boolean | null;
  failed: boolean;
}

export const INITIAL_TERMINAL: TerminalState = { screen: null, running: null, failed: false };

export function afterRead(_state: TerminalState, result: string | null): TerminalState {
  if (result === null) return { screen: null, running: false, failed: false };
  return { screen: result, running: true, failed: false };
}

export function afterFailure(state: TerminalState): TerminalState {
  return { ...state, failed: true };
}

export function showsReadFailed(state: TerminalState): boolean {
  return state.failed && state.screen !== null;
}
