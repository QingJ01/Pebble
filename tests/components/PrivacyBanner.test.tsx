import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { RenderedHtml } from "../../src/lib/api";
import PrivacyBanner from "../../src/components/PrivacyBanner";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string | Record<string, unknown>) =>
      typeof fallback === "string" ? fallback : key,
  }),
}));

function rendered(trackers: number, images: number): RenderedHtml {
  return {
    html: "",
    trackers_blocked: Array.from({ length: trackers }, () => ({ domain: "t.example.com", tracker_type: "pixel" })),
    images_blocked: images,
  };
}

describe("PrivacyBanner", () => {
  it("shows no trust actions when only trackers are blocked", () => {
    render(<PrivacyBanner rendered={rendered(1, 0)} onLoadImages={vi.fn()} onTrustSender={vi.fn()} />);

    // Trusting a sender never unblocks trackers, so offering it would do nothing.
    expect(screen.getByRole("region")).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("offers load and trust actions when images are blocked", () => {
    const onTrustSender = vi.fn();
    render(<PrivacyBanner rendered={rendered(1, 2)} onLoadImages={vi.fn()} onTrustSender={onTrustSender} />);

    fireEvent.click(screen.getByRole("button", { name: "Trust sender" }));
    expect(onTrustSender).toHaveBeenCalledWith("all");
    expect(screen.getByRole("button", { name: "Trust images" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "privacy.loadImages" })).toBeTruthy();
  });

  it("renders nothing when nothing is blocked", () => {
    const { container } = render(
      <PrivacyBanner rendered={rendered(0, 0)} onLoadImages={vi.fn()} onTrustSender={vi.fn()} />,
    );
    expect(container.firstChild).toBeNull();
  });
});
