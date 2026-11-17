export function answerBody(option: string, text: string): string {
  const note = text.trim();
  return note === "" ? option : `${option}\n\n${note}`;
}
