import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(process.cwd(), "src", "components", "mail-list.css"), "utf8");

describe("mail row hover actions", () => {
  it("use 24px buttons so the bar fits the sender line", () => {
    expect(css).toMatch(/\.message-row-actions \.icon-btn\s*\{[^}]*width:\s*var\(--row-action-size\)/);
    expect(css).toMatch(/\.message-row-actions\s*\{[^}]*--row-action-size:\s*24px/);
  });

  it("make the first line leave room for the bar instead of drawing over the text", () => {
    expect(css).toMatch(/\.message-list-row:hover > \.mail-row__line[\s\S]*?\{[^}]*padding-right:/);
    expect(css).toMatch(/\.message-list-row:hover \.mail-row__meta[\s\S]*?\{[^}]*display:\s*none/);
  });
});
