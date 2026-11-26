import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("Tauri startup window config", () => {
  it("keeps the main window hidden until the frontend shows it", () => {
    const configPath = resolve(process.cwd(), "src-tauri", "tauri.conf.json");
    const config = JSON.parse(readFileSync(configPath, "utf8"));
    const mainWindow = config.app.windows.find((windowConfig: { label?: string }) => {
      return windowConfig.label === "main";
    });

    expect(mainWindow.visible).toBe(false);
    expect(mainWindow.backgroundColor).toBe("#1a1a1a");
  });

  it("creates the macOS window with the overlay title bar from the start", () => {
    const read = (file: string) =>
      JSON.parse(readFileSync(resolve(process.cwd(), "src-tauri", file), "utf8"));
    const main = (config: { app: { windows: { label?: string }[] } }) =>
      config.app.windows.find((windowConfig) => windowConfig.label === "main") as Record<string, unknown>;
    const base = main(read("tauri.conf.json"));
    const mac = main(read("tauri.macos.conf.json"));

    // Native title hidden and traffic lights centred on the 36px title bar
    // (y is roughly the buttons' centre from the window top).
    expect(mac).toMatchObject({
      decorations: true,
      titleBarStyle: "Overlay",
      hiddenTitle: true,
      trafficLightPosition: { x: 12, y: 18 },
    });
    // The platform file replaces the whole windows array: everything else
    // must stay in step with the main config.
    const macOnly = ["decorations", "titleBarStyle", "hiddenTitle", "trafficLightPosition"];
    const strip = (w: Record<string, unknown>) =>
      Object.fromEntries(Object.entries(w).filter(([key]) => !macOnly.includes(key)));
    expect(strip(mac)).toEqual(strip(base));
  });

  it("allows the frontend to show the hidden main window", () => {
    const capabilityPath = resolve(process.cwd(), "src-tauri", "capabilities", "default.json");
    const capability = JSON.parse(readFileSync(capabilityPath, "utf8"));

    expect(capability.permissions).toContain("core:window:allow-show");
  });

  it("allows close-request handlers to finish closing the main window", () => {
    const capabilityPath = resolve(process.cwd(), "src-tauri", "capabilities", "default.json");
    const capability = JSON.parse(readFileSync(capabilityPath, "utf8"));

    expect(capability.permissions).toContain("core:window:allow-close");
    expect(capability.permissions).toContain("core:window:allow-destroy");
  });
});
