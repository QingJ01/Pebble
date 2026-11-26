import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import Button from "../../../src/components/ui/Button";
import IconButton from "../../../src/components/ui/IconButton";
import { documentLangFor } from "../../../src/lib/i18n";

describe("Button", () => {
  it("applies variant/size classes and defaults to type=button", () => {
    render(<Button variant="danger" size="sm">Delete</Button>);
    const button = screen.getByRole("button", { name: "Delete" });
    expect(button.className).toBe("btn btn--danger btn--sm");
    expect(button.getAttribute("type")).toBe("button");
  });

  it("stays focusable but inert while loading", () => {
    const onClick = vi.fn();
    render(<Button loading onClick={onClick}>Save</Button>);
    const button = screen.getByRole("button", { name: "Save" });
    expect(button.hasAttribute("disabled")).toBe(false);
    expect(button.getAttribute("aria-busy")).toBe("true");
    expect(button.getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });
});

describe("IconButton", () => {
  it("requires a label used for aria-label and the tooltip, and exposes pressed state", () => {
    render(<IconButton label="Star" icon={<svg />} pressed={false} />);
    const button = screen.getByRole("button", { name: "Star" });
    expect(button.getAttribute("title")).toBe("Star");
    expect(button.getAttribute("aria-pressed")).toBe("false");
    expect(button.className).toContain("icon-btn--sm");
  });

  it("omits aria-pressed for non-toggle buttons", () => {
    render(<IconButton label="Close" icon={<svg />} tooltip={false} />);
    const button = screen.getByRole("button", { name: "Close" });
    expect(button.hasAttribute("aria-pressed")).toBe(false);
    expect(button.hasAttribute("title")).toBe(false);
  });
});

describe("document language", () => {
  it("maps the app language to an html lang value", () => {
    expect(documentLangFor("zh")).toBe("zh-CN");
    expect(documentLangFor("en")).toBe("en");
    expect(document.documentElement.lang).toMatch(/^(en|zh-CN)$/);
  });
});
