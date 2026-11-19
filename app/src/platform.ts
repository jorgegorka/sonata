export const isMac = typeof navigator === "undefined" || /Mac|iPhone|iPad/.test(navigator.platform);

export function platformStrings<T extends Record<string, unknown>>(strings: T, mac = isMac): T {
  const { linux, ...shared } = strings;
  const result: Record<string, unknown> = { ...shared };
  if (!mac && linux) {
    for (const [section, overrides] of Object.entries(linux as Record<string, Record<string, string>>)) {
      result[section] = { ...(result[section] as Record<string, unknown>), ...overrides };
    }
  }
  return result as T;
}
