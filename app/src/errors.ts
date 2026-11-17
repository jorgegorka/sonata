import type { TFunction } from "i18next";

export interface CommandError {
  code: string;
  params?: Record<string, string>;
}

export function isCommandError(e: unknown): e is CommandError {
  return typeof e === "object" && e !== null && typeof (e as { code?: unknown }).code === "string";
}

export function failUnless(code: string, fail: (e: unknown) => void): (e: unknown) => void {
  return (e) => {
    if (!(isCommandError(e) && e.code === code)) fail(e);
  };
}

export function errorMessage(t: TFunction, e: unknown): string {
  if (isCommandError(e)) {
    return t(`errors.${e.code}`, { ...(e.params ?? {}), defaultValue: t("errors.generic", { error: e.code }) });
  }
  return t("errors.generic", { error: String(e) });
}

export function systemParams(t: TFunction, params: Record<string, string> | null): Record<string, string> {
  const p = params ?? {};
  if (typeof p.code !== "string") return p;
  return { ...p, error: errorMessage(t, { code: p.code, params: p }) };
}
