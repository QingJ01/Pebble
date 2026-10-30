import { fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ShadowDomEmail } from "@/components/ShadowDomEmail";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  openMailtoUrl: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@/app/useMailtoOpen", () => ({
  openMailtoUrl: mocks.openMailtoUrl,
}));

describe("ShadowDomEmail", () => {
  afterEach(() => {
    document.documentElement.removeAttribute("data-theme");
  });

  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.openMailtoUrl.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
    mocks.openMailtoUrl.mockResolvedValue(true);
  });

  it("uses app theme variables instead of hardcoded light text styles", async () => {
    document.documentElement.setAttribute("data-theme", "dark");

    const { container } = render(<ShadowDomEmail html="<p>Hello</p>" />);
    const host = container.firstChild as HTMLDivElement | null;

    await waitFor(() => {
      expect(host?.shadowRoot).not.toBeNull();
    });

    const shadowMarkup = host!.shadowRoot!.innerHTML;
    expect(shadowMarkup).toContain("var(--color-text-primary)");
    expect(shadowMarkup).toContain("a[href] { color: var(--color-email-link); }");
    expect(shadowMarkup).not.toContain("color: #1a1a1a");
  });

  it("themes horizontal overflow inside email content", async () => {
    const { container } = render(<ShadowDomEmail html="<pre>long code line</pre>" />);
    const host = container.firstChild as HTMLDivElement | null;

    await waitFor(() => {
      expect(host?.shadowRoot).not.toBeNull();
    });

    const shadowMarkup = host!.shadowRoot!.innerHTML;
    expect(shadowMarkup).toContain("scrollbar-width: thin");
    expect(shadowMarkup).toContain("::-webkit-scrollbar-thumb");
  });

  it("keeps light-authored email html readable in dark theme", async () => {
    document.documentElement.setAttribute("data-theme", "dark");

    const { container } = render(
      <ShadowDomEmail html={'<div style="color: #000000">Dark inline text</div>'} />,
    );
    const host = container.firstChild as HTMLDivElement | null;

    await waitFor(() => {
      expect(host?.shadowRoot).not.toBeNull();
    });

    const shadowMarkup = host!.shadowRoot!.innerHTML;
    expect(shadowMarkup).toContain('class="pebble-email-content"');
    expect(shadowMarkup).not.toContain(":host-context");
    expect(shadowMarkup).toContain("color-scheme: var(--email-content-color-scheme)");
    expect(shadowMarkup).toContain("background: var(--email-content-bg)");
    expect(shadowMarkup).toContain("color: var(--email-content-color)");
    expect(shadowMarkup).toContain("display: var(--email-content-display)");
  });

  it("preserves sender-authored link and button colors", () => {
    const { container } = render(
      <ShadowDomEmail html={'<a href="https://example.com" style="color: blue">Link</a><a href="https://example.com" style="background: blue; color: white"><span style="color: white">Button</span></a>'} />,
    );
    const shadow = (container.firstChild as HTMLDivElement).shadowRoot!;
    const anchors = shadow.querySelectorAll("a");

    expect(anchors[0].style.color).toBe("blue");
    expect(anchors[1].style.background).toBe("blue");
    expect(anchors[1].style.color).toBe("white");
    expect(anchors[1].querySelector("span")!.style.color).toBe("white");
  });

  it("prevents full-height email wrappers from painting a gray viewport canvas", async () => {
    const html = `
      <table height="100%" style="height: 100%; background: #f1f1f1">
        <tbody><tr><td>Cloudflare content</td></tr></tbody>
      </table>
    `;

    const { container } = render(<ShadowDomEmail html={html} />);
    const host = container.firstChild as HTMLDivElement | null;

    await waitFor(() => {
      expect(host?.shadowRoot?.querySelector(".pebble-email-content")).not.toBeNull();
    });

    const shadowMarkup = host!.shadowRoot!.innerHTML;
    expect(shadowMarkup).toContain('.pebble-email-content > table[height="100%"]');
    expect(shadowMarkup).toContain('style="height: 100%; background: #f1f1f1"');
    expect(shadowMarkup).toContain("height: auto !important");
    expect(shadowMarkup).toContain("min-height: 0 !important");
  });

  it("renders approved email CSS inside the shadow content", async () => {
    const html = `
      <style>.hero { color: red; }</style>
      <link rel="stylesheet" href="https://cdn.example.com/mail.css">
      <p class="hero">Styled body</p>
    `;

    const { container } = render(<ShadowDomEmail html={html} />);
    const host = container.firstChild as HTMLDivElement | null;

    await waitFor(() => {
      expect(host?.shadowRoot?.querySelector(".pebble-email-content")).not.toBeNull();
    });

    const content = host!.shadowRoot!.querySelector(".pebble-email-content")!;
    expect(content.querySelector("style")?.textContent).toContain(".hero");
    expect(content.querySelector("link")?.getAttribute("href")).toBe("https://cdn.example.com/mail.css");
    expect(content.querySelector(".hero")?.textContent).toBe("Styled body");
  });

  it("opens http and https links through the external URL command", async () => {
    const { container } = render(
      <ShadowDomEmail html={'<a href="http://pebble.byebug.cn/">Pebble</a>'} />,
    );
    const host = container.firstChild as HTMLDivElement | null;

    await waitFor(() => {
      expect(host?.shadowRoot?.querySelector("a")).not.toBeNull();
    });

    fireEvent.click(host!.shadowRoot!.querySelector("a")!);

    expect(mocks.invoke).toHaveBeenCalledWith("open_external_url", {
      url: "http://pebble.byebug.cn/",
    });
  });

  it("opens mailto links through the compose mailto handler", async () => {
    const { container } = render(
      <ShadowDomEmail html={'<a href="mailto:qingj1314@163.com">qingj1314@163.com</a>'} />,
    );
    const host = container.firstChild as HTMLDivElement | null;

    await waitFor(() => {
      expect(host?.shadowRoot?.querySelector("a")).not.toBeNull();
    });

    fireEvent.click(host!.shadowRoot!.querySelector("a")!);

    expect(mocks.openMailtoUrl).toHaveBeenCalledWith("mailto:qingj1314@163.com");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
