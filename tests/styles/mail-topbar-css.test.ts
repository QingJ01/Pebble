import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");

describe("mail top bars stay one row", () => {
  it("never wraps the inbox or search top bar", () => {
    expect(css).toMatch(/\.search-toolbar\s*\{[^}]*flex-wrap\s*:\s*nowrap/i);
    expect(css).toMatch(/\.inbox-toolbar-row\s*\{[^}]*flex-wrap\s*:\s*nowrap/i);
    // No breakpoint turns wrapping back on or pushes the shell to a full line.
    expect(css).not.toMatch(/\.(search-toolbar|inbox-toolbar-row)[^{]*\{[^}]*flex-wrap\s*:\s*wrap/i);
    expect(css).not.toMatch(/\.search-input-shell\s*\{[^}]*flex-basis\s*:\s*100%/i);
  });

  it("lets the search shell shrink to 160px while the buttons keep their size", () => {
    expect(css).toMatch(/\.search-input-shell\s*\{[^}]*min-width\s*:\s*160px/i);
    expect(css).toMatch(/\.inbox-toolbar-search\s*\{[^}]*min-width\s*:\s*min-content/i);
    expect(css).toMatch(
      /\.search-toolbar > :not\(\.search-input-shell\),\s*\.inbox-toolbar-row > :not\(\.inbox-toolbar-search\)\s*\{[^}]*flex-shrink\s*:\s*0/i,
    );
  });
});
