import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("email content theme", () => {
  const css = readFileSync(join(process.cwd(), "src", "styles", "index.css"), "utf8");
  const light = css.match(/:root\s*\{(\s*color-scheme: light;[^}]+)\}/)![1];
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
    expect(dark).not.toContain("--color-email-link:");
  });

  it("shows dark-mode HTML mail as a full-width padded light card", () => {
    // display stays block (inherited from :root) instead of an inline-block box sized to content.
    expect(dark).not.toContain("inline-block");
    expect(dark).toContain("--email-content-padding: 16px");
    expect(dark).toContain("--email-content-radius: 8px");
    expect(light).toContain("--email-content-padding: 0");
  });
});
