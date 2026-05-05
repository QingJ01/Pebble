import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("focus-visible CSS", () => {
  it("does not suppress the Tiptap editor focus outline", () => {
    const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");

    expect(css).not.toMatch(/\.tiptap\s*\{[^}]*outline\s*:\s*none/i);
    expect(css).not.toMatch(/\.tiptap:focus\s*\{[^}]*outline\s*:\s*none/i);
  });

  it("uses a custom themed checkbox for batch selection", () => {
    const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");

    expect(css).toMatch(/\.batch-checkbox\s*\{[^}]*appearance\s*:\s*none/i);
    expect(css).toMatch(/\.batch-checkbox:checked\s*\{[^}]*background\s*:\s*var\(--color-accent\)/i);
    expect(css).toMatch(/\.batch-checkbox::before\s*\{[^}]*border-left\s*:/i);
  });

  it("themes native form controls in dark mode", () => {
    const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");

    expect(css).toMatch(/input,\s*textarea,\s*select,\s*option\s*\{[^}]*color-scheme\s*:\s*light dark/i);
    expect(css).toMatch(/input\[type="checkbox"\],\s*input\[type="radio"\]\s*\{[^}]*accent-color\s*:\s*var\(--color-accent\)/i);
    expect(css).toMatch(/\[data-theme="dark"\]\s*input,\s*\[data-theme="dark"\]\s*textarea,\s*\[data-theme="dark"\]\s*select,\s*\[data-theme="dark"\]\s*option\s*\{[^}]*color-scheme\s*:\s*dark/i);
    expect(css).toMatch(/\[data-theme="dark"\]\s*select,\s*\[data-theme="dark"\]\s*option\s*\{[^}]*background-color\s*:\s*var\(--color-bg\)/i);
    expect(css).toMatch(/\[data-theme="dark"\]\s*select,\s*\[data-theme="dark"\]\s*option\s*\{[^}]*color\s*:\s*var\(--color-text-primary\)/i);
    expect(css).toMatch(/\[data-theme="dark"\]\s*input\[type="date"\]::-webkit-calendar-picker-indicator\s*\{[^}]*filter\s*:\s*invert\(1\)/i);
  });

  it("makes unread message and thread rows visually distinct in dark mode", () => {
    const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");

    expect(css).toMatch(/\.message-list-row--unread\[aria-selected="false"\]:hover/i);
  });
});
