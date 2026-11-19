export type LinkAction = "inline" | "external" | "ignore";

export function linkAction(href: string | undefined, modifiers: { metaKey: boolean; ctrlKey?: boolean }): LinkAction {
  if (!href) return "ignore";
  let protocol: string;
  try {
    ({ protocol } = new URL(href));
  } catch {
    return "ignore";
  }
  if (protocol === "mailto:") return "external";
  if (protocol === "http:" || protocol === "https:") return (modifiers.metaKey || modifiers.ctrlKey) ? "external" : "inline";
  return "ignore";
}
