import { fireEvent, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
    expect(shadowMarkup).toContain("var(--color-accent)");
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
    expect(shadowMarkup).toContain(':host-context([data-theme="dark"]) .pebble-email-content');
    expect(shadowMarkup).toContain("color-scheme: light");
    expect(shadowMarkup).toContain("background: #fff");
    expect(shadowMarkup).toContain("color: #202124");
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
