import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("search controls CSS", () => {
  it("gives the inbox search field a distinct input surface", () => {
    const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");

    expect(css).toMatch(/\.search-input-shell\s*\{[^}]*height\s*:\s*var\(--control-lg\)/i);
    expect(css).toMatch(/\.search-input-shell\s*\{[^}]*border-radius\s*:\s*var\(--radius-md\)/i);
    expect(css).toMatch(/\.search-input-shell\s*\{[^}]*transition\s*:[^;]*box-shadow/i);
    expect(css).toMatch(/\.search-input-shell:focus-within\s*\{[^}]*border-color\s*:\s*var\(--color-focus-ring\)/i);
    expect(css).toMatch(
      /\.app-shell--with-background\s+\.search-input-shell\s*\{[^}]*color-mix\(in srgb,\s*var\(--color-canvas\)\s*18%,\s*transparent\)/i,
    );
    expect(css).toMatch(
      /\.app-shell--with-background\s+\.search-toolbar\s*\{[^}]*color-mix\(in srgb,\s*var\(--color-canvas\)\s*12%,\s*transparent\)/i,
    );
  });
});
