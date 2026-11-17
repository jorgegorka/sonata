export function formatReset(iso: string, now: Date, language: string): string {
  const at = new Date(iso);
  const time = new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(at);
  if (at.toDateString() === now.toDateString()) return time;
  const date = new Intl.DateTimeFormat(language, { month: "short", day: "numeric" }).format(at);
  return `${date}, ${time}`;
}
