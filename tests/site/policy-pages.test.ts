import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const siteDir = join(process.cwd(), "site");

function readSiteFile(name: string) {
  return readFileSync(join(siteDir, name), "utf8");
}

describe("public policy pages", () => {
  it("provides public privacy policy and terms pages for OAuth review", () => {
    expect(existsSync(join(siteDir, "privacy.html"))).toBe(true);
    expect(existsSync(join(siteDir, "terms.html"))).toBe(true);
  });

  it("links privacy policy and terms from the public site footer", () => {
    const index = readSiteFile("index.html");

    expect(index).toContain('href="privacy.html"');
    expect(index).toContain('href="terms.html"');
  });

  it("discloses Google user data handling in the privacy policy", () => {
    const privacy = readSiteFile("privacy.html");

    expect(privacy).toContain("Google user data");
    expect(privacy).toContain("https://mail.google.com/");
    expect(privacy).toContain("OAuth tokens");
    expect(privacy).toContain("Limited Use");
  });

  it("wires privacy and terms pages into the site i18n system", () => {
    const privacy = readSiteFile("privacy.html");
    const terms = readSiteFile("terms.html");
    const script = readSiteFile("main.js");

    for (const page of [privacy, terms]) {
      expect(page).toContain("lang-switch");
      expect(page).toContain('data-lang="en"');
      expect(page).toContain('data-lang="zh"');
      expect(page).toContain('<script src="main.js"></script>');
    }

    expect(privacy).toContain('data-i18n="privacyPolicyTitle"');
    expect(terms).toContain('data-i18n="termsTitle"');
    expect(script).toContain("privacyPolicyTitle");
    expect(script).toContain("termsTitle");
  });
});
