import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("email content theme", () => {
  const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");
  const light = css.match(/:root\s*\{([^}]+)\}/)![1];
  const dark = css.match(/\[data-theme="dark"\]\s*\{([^}]+)\}/)![1];

  it("pairs readable default links with the light email canvas", () => {
    expect(light).toContain("--color-email-link: #1d4ed8");
    expect(light).toContain("--email-content-color-scheme: light");
    expect(light).toContain("--email-content-bg: transparent");
    expect(light).toContain("--email-content-color: inherit");
    expect(light).toContain("--email-content-display: block");
  });

  it("uses inherited properties to protect authored colors in dark-mode webviews", () => {
    expect(dark).toContain("--email-content-bg: #fff");
    expect(dark).toContain("--email-content-color: #202124");
    expect(dark).toContain("--email-content-display: inline-block");
    expect(dark).not.toContain("--color-email-link:");
  });
});
