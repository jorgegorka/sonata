import type { Provider, ProviderStatus } from "./api";

export type ProviderState = "ready" | "unchecked" | "notLoggedIn" | "missing";

export function providerState(status: ProviderStatus): ProviderState {
  if (status.path === null) return "missing";
  if (status.logged_in === false) return "notLoggedIn";
  return status.logged_in === true ? "ready" : "unchecked";
}

export function availableProviders(statuses: ProviderStatus[] | null, all: Provider[]): Provider[] {
  if (statuses === null) return all;
  return all.filter((p) => statuses.some((s) => s.provider === p && s.path !== null));
}
