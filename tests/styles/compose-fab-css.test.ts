import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("compose FAB CSS", () => {
  it("stays fixed even though app-shell children are made position: relative", () => {
    const shellCss = readFileSync(join(process.cwd(), "src", "app", "shell.css"), "utf8");
    const indexCss = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");

    // index.css gives every direct child of .app-shell position: relative with
    // specificity (0,2,0); a bare `.compose-fab` (0,1,0) would lose and the FAB
    // would render at the bottom-left, half off-screen.
    expect(indexCss).toMatch(/\.app-shell\s*>\s*:not\(\.app-background-layer\)\s*\{[^}]*position:\s*relative/);
    expect(shellCss).toMatch(/\.app-shell\s*>\s*\.compose-fab[\s\S]*?\{[^}]*position:\s*fixed/);
  });
});
