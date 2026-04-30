import { describe, expect, it } from "vitest";
import { sanitizeHtml } from "../../src/lib/sanitizeHtml";

describe("sanitizeHtml", () => {
  it("preserves safe inline email styles", () => {
    const sanitized = sanitizeHtml(
      '<p style="color: red; text-align: center; margin: 8px">Hello</p>',
    );

    expect(sanitized).toContain("style=");
    expect(sanitized).toContain("color:");
    expect(sanitized).toContain("text-align:");
  });

  it("removes unsafe inline style content", () => {
    const sanitized = sanitizeHtml(
      '<p style="background-image: url(javascript:alert(1)); color: blue">Hello</p>',
    );

    expect(sanitized).not.toContain("javascript:");
    expect(sanitized).toContain("color:");
  });

  it("removes inline styles with escaped url tokens", () => {
    const sanitized = sanitizeHtml(
      `<p style="color: u\\72l('https://evil.example/track')">hello</p>`,
    );

    expect(sanitized).not.toContain("evil.example");
    expect(sanitized).not.toContain("u\\72l");
  });
});
