import { describe, expect, it } from "vitest";
import { darkenSimpleEmail, FORCE_DARK_MEDIA_SELECTOR, parseCssColor, relativeLuminance, saturation } from "@/lib/emailDarkMode";

function mount(html: string) {
  const root = document.createElement("div");
  root.innerHTML = html;
  document.body.appendChild(root);
  return root;
}

describe("color helpers", () => {
  it("parses computed rgb/rgba colors", () => {
    expect(parseCssColor("rgb(255, 255, 255)")).toEqual([255, 255, 255, 1]);
    expect(parseCssColor("rgba(0, 0, 0, 0)")).toEqual([0, 0, 0, 0]);
    expect(parseCssColor("rgb(10 20 30 / 50%)")).toEqual([10, 20, 30, 0.5]);
    expect(parseCssColor("transparent")).toBeNull();
  });

  it("measures luminance and saturation", () => {
    expect(relativeLuminance([255, 255, 255, 1])).toBeCloseTo(1);
    expect(relativeLuminance([0, 0, 0, 1])).toBe(0);
    expect(saturation([128, 128, 128, 1])).toBe(0);
    expect(saturation([200, 0, 0, 1])).toBeCloseTo(1);
  });
});

describe("darkenSimpleEmail", () => {
  it("clears white backgrounds and lifts dark text in simple mail", () => {
    const root = mount(`
      <table style="background-color:#ffffff"><tr><td>
        <p id="gray" style="color:#333333">Hello</p>
        <p id="red" style="color:#990000">Alert</p>
      </td></tr></table>`);
    expect(darkenSimpleEmail(root)).toBe(true);
    expect(root.querySelector("table")!.style.getPropertyValue("background-color")).toBe("transparent");
    expect(root.querySelector<HTMLElement>("#gray")!.style.getPropertyValue("color")).toBe("var(--color-text-primary)");
    expect(root.querySelector<HTMLElement>("#red")!.style.getPropertyValue("color")).toContain("color-mix");
  });

  it("leaves designed mail untouched", () => {
    const root = mount(`
      <div style="background-color:#0b5394"><p style="color:#333">Banner</p></div>`);
    expect(darkenSimpleEmail(root)).toBe(false);
    expect(root.querySelector("p")!.style.getPropertyValue("color")).toBe("rgb(51, 51, 51)");
  });
});

describe("FORCE_DARK_MEDIA_SELECTOR", () => {
  it("restores each piece of media once", () => {
    const root = mount(`
      <picture><img id="p"></picture>
      <table><tr><td background="x.jpg" id="bg"><img id="inbg"></td></tr></table>
      <div style="background-image:url(x)" id="bgs"><span><img id="inbgs"></span></div>
      <img id="plain">`);
    const hit = [...root.querySelectorAll(FORCE_DARK_MEDIA_SELECTOR)].map((e) => e.id).sort();
    expect(hit).toEqual(["bg", "bgs", "p", "plain"]);
  });
});
