import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("mailto deep-link integration", () => {
  it("registers mailto as a desktop deep-link scheme", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );

    expect(config.plugins["deep-link"].desktop.schemes).toContain("mailto");
  });

  it("initializes deep-link and single-instance plugins in Rust", () => {
    const cargoToml = readFileSync(resolve(process.cwd(), "src-tauri", "Cargo.toml"), "utf8");
    const source = readFileSync(resolve(process.cwd(), "src-tauri", "src", "lib.rs"), "utf8");

    expect(cargoToml).toContain("tauri-plugin-deep-link");
    expect(cargoToml).toContain("tauri-plugin-single-instance");
    expect(cargoToml).toContain('features = ["deep-link"]');
    expect(source).toContain("tauri_plugin_single_instance::init");
    expect(source).toContain("tauri_plugin_deep_link::init()");
    expect(source).toContain("deep-link://new-url");
    expect(source).toContain("take_pending_mailto_urls");
  });

  it("lets the deep-link plugin own single-instance URL dispatch", () => {
    const source = readFileSync(resolve(process.cwd(), "src-tauri", "src", "lib.rs"), "utf8");

    expect(source).not.toContain("record_mailto_urls(app, mailto_urls_from_args(&args))");
  });
});
