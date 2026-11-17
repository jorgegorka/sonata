export function fileName(path: string): string {
  const parts = path.split("/").filter((part) => part.length > 0);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

export function addFiles(current: string[], added: string[]): string[] {
  const next = current.slice();
  for (const path of added) if (!next.includes(path)) next.push(path);
  return next;
}

export const RUNNABLE_EXTENSIONS = [
  "app", "applescript", "bash", "command", "csh", "fileloc", "fish", "inetloc", "jar", "ksh", "mobileconfig",
  "mpkg", "pkg", "pl", "py", "rb", "scpt", "sh", "terminal", "tool", "webloc", "workflow", "zsh",
];

export function opensPrograms(name: string): boolean {
  const dot = name.lastIndexOf(".");
  return dot > 0 && RUNNABLE_EXTENSIONS.includes(name.slice(dot + 1).toLowerCase());
}
