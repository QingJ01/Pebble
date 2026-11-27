import { describe, expect, it } from "vitest";
import { MAIL_PROVIDERS, providerById, providerForEmail, QUICK_SETUP_PROVIDER_IDS } from "@/lib/mailProviders";

describe("providerForEmail", () => {
  it("matches exact domains case-insensitively", () => {
    expect(providerForEmail("Someone@QQ.com")?.id).toBe("qq");
    expect(providerForEmail("a@hotmail.co.uk")?.id).toBe("outlook");
    expect(providerForEmail("a@me.com")?.settings.imap_host).toBe("imap.mail.me.com");
  });

  it("ignores unknown, partial and malformed addresses", () => {
    expect(providerForEmail("a@company.com")).toBeNull();
    expect(providerForEmail("a@mail.qq.com")).toBeNull();
    expect(providerForEmail("qq.com")).toBeNull();
    expect(providerForEmail("a@")).toBeNull();
  });

  it("maps every domain to one provider with consistent ports", () => {
    const seen = new Set<string>();
    for (const provider of MAIL_PROVIDERS) {
      for (const domain of provider.domains) {
        expect(seen.has(domain), domain).toBe(false);
        expect(domain).toBe(domain.toLowerCase());
        seen.add(domain);
      }
      const { smtp_port, smtp_security, imap_port, imap_security } = provider.settings;
      expect(imap_port === 993 && imap_security === "tls").toBe(true);
      expect(smtp_security).toBe(smtp_port === 587 ? "starttls" : "tls");
    }
  });

  it("keeps the quick-setup buttons resolvable", () => {
    for (const id of QUICK_SETUP_PROVIDER_IDS) expect(providerById(id)).toBeDefined();
  });
});
