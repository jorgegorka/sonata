const WHITE = "#ffffff";
const DARK = "#111827";

function textColor(background: string, minimum: number): string {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(background);
  if (!match) return WHITE;
  const [r, g, b] = match.slice(1).map((hex) => {
    const c = parseInt(hex, 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  if (1.05 / (luminance + 0.05) >= minimum) return WHITE;
  // #111827 is the existing avatar foreground. Midtone accents occasionally
  // need black to meet the higher contrast requirement for normal button text.
  const darkLuminance = 0.009189219295824108;
  return (luminance + 0.05) / (darkLuminance + 0.05) >= minimum ? DARK : "#000000";
}

export const avatarTextColor = (background: string): string => textColor(background, 3);
export const accentTextColor = (background: string): string => textColor(background, 4.5);
