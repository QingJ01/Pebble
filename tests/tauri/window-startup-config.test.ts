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

  it("allows the frontend to show the hidden main window", () => {
    const capabilityPath = resolve(process.cwd(), "src-tauri", "capabilities", "default.json");
    const capability = JSON.parse(readFileSync(capabilityPath, "utf8"));

    expect(capability.permissions).toContain("core:window:allow-show");
  });
});
