/**
 * "Darken simple mail" (#103): HTML mail that only uses white or transparent
 * backgrounds can follow the dark theme by clearing those backgrounds and
 * lifting dark text. Mail with designed backgrounds (colored blocks, images)
 * is left alone and keeps the light card.
 */

/**
 * "Always dark": media restored after the mail is inverted. Each piece is
 * restored once; media inside an already restored background block is left
 * alone, or the filters would stack back into a negative.
 */
export const FORCE_DARK_MEDIA_SELECTOR =
  ':is(img, video, canvas, [background], [style*="background-image" i])'
  + ':not(:is([background], [style*="background-image" i]) *)';

/** Past this many elements the mail is treated as designed (and not walked). */
const MAX_ELEMENTS = 4000;

export type Rgba = [number, number, number, number];

/** Parse a computed `rgb()` / `rgba()` color; null for anything else. */
export function parseCssColor(value: string): Rgba | null {
  const match = value.trim().match(/^rgba?\(([^)]+)\)$/i);
  if (!match) return null;
  const parts = match[1].split(/[\s,/]+/).filter(Boolean);
  if (parts.length < 3) return null;
  const channel = (part: string) => part.endsWith("%")
    ? (parseFloat(part) / 100) * 255
    : parseFloat(part);
  const [r, g, b] = parts.slice(0, 3).map(channel);
  const alphaPart = parts[3];
  const a = alphaPart === undefined
    ? 1
    : alphaPart.endsWith("%") ? parseFloat(alphaPart) / 100 : parseFloat(alphaPart);
  if ([r, g, b, a].some((n) => Number.isNaN(n))) return null;
  return [r, g, b, a];
}

/** WCAG relative luminance, 0 (black) to 1 (white). */
export function relativeLuminance([r, g, b]: Rgba): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** HSL saturation, 0 (gray) to 1. */
export function saturation([r, g, b]: Rgba): number {
  const max = Math.max(r, g, b) / 255;
  const min = Math.min(r, g, b) / 255;
  if (max === min) return 0;
  const l = (max + min) / 2;
  return (max - min) / (1 - Math.abs(2 * l - 1));
}

const isTransparent = (color: Rgba) => color[3] < 0.05;
/** White-ish page backgrounds that only exist to sit on a light canvas. */
const isPlainLightBackground = (color: Rgba) => relativeLuminance(color) > 0.85;

/**
 * Restyle `root`'s mail for a dark canvas if it is simple enough. Returns
 * false (leaving the DOM untouched) when the mail has designed backgrounds.
 */
export function darkenSimpleEmail(root: HTMLElement, view: Window = window): boolean {
  const elements = [root, ...Array.from(root.querySelectorAll<HTMLElement>("*"))];
  if (elements.length > MAX_ELEMENTS) return false;

  const clearBackground: HTMLElement[] = [];
  const recolor: { element: HTMLElement; color: Rgba }[] = [];
  for (const element of elements) {
    if (element instanceof HTMLImageElement || element instanceof SVGElement) continue;
    const style = view.getComputedStyle(element);
    const image = style.backgroundImage;
    if (image && image !== "none") return false;
    const background = parseCssColor(style.backgroundColor);
    if (background && !isTransparent(background)) {
      if (!isPlainLightBackground(background)) return false;
      if (element !== root) clearBackground.push(element);
    }
    const color = parseCssColor(style.color);
    if (color && relativeLuminance(color) < 0.4) recolor.push({ element, color });
  }

  for (const element of clearBackground) {
    element.style.setProperty("background-color", "transparent", "important");
  }
  for (const { element, color } of recolor) {
    if (element === root) continue;
    // Gray/black text takes the theme's text color; colored text is lifted
    // toward white so it keeps its hue but reads on the dark canvas.
    const value = saturation(color) < 0.25
      ? "var(--color-text-primary)"
      : `color-mix(in srgb, rgb(${color[0]} ${color[1]} ${color[2]}) 55%, white)`;
    element.style.setProperty("color", value, "important");
  }
  return true;
}
