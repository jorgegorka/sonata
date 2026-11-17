export default function PanelGlyph({ side }: { side: "left" | "right" }) {
  const pane =
    side === "left"
      ? "M3 0.75h3v12.5H3A2.25 2.25 0 0 1 0.75 11V3A2.25 2.25 0 0 1 3 0.75z"
      : "M13 0.75h-3v12.5h3A2.25 2.25 0 0 0 15.25 11V3A2.25 2.25 0 0 0 13 0.75z";
  return (
    <svg width="16" height="14" viewBox="0 0 16 14" aria-hidden="true">
      <rect
        x="0.75"
        y="0.75"
        width="14.5"
        height="12.5"
        rx="2.25"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path d={pane} fill="currentColor" />
    </svg>
  );
}
