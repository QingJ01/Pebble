import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("background image asset protocol config", () => {
  it("enables the asset protocol for imported background images", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );

    expect(config.app.security.assetProtocol).toMatchObject({
      enable: true,
      scope: ["$APPDATA/backgrounds/**/*"],
    });
    expect(config.app.security.csp).toContain("asset:");
    expect(config.app.security.csp).toContain("http://asset.localhost");
  });

  it("allows unrestricted privacy mode to load remote email stylesheets", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );

    expect(config.app.security.csp).toMatch(/style-src[^;]*https:/);
  });

  it("keeps unsafe-inline effective for email styles by not injecting style-src nonces", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );

    // Tauri adds nonces/hashes to modified directives, and browsers ignore
    // 'unsafe-inline' when a nonce is present, blocking shadow-DOM <style>
    // and email style attributes in packaged builds.
    expect(config.app.security.csp).toMatch(/style-src[^;]*'unsafe-inline'/);
    expect(config.app.security.dangerousDisableAssetCspModification).toEqual(["style-src"]);
  });

  it("allows remote http images used by some email senders", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );

    expect(config.app.security.csp).toMatch(/img-src[^;]*(^|\s)http:(\s|;)/);
  });

  it("enables the matching tauri protocol-asset cargo feature", () => {
    const cargo = readFileSync(resolve(process.cwd(), "Cargo.toml"), "utf8");

    expect(cargo).toMatch(/tauri\s*=\s*\{[^}]*features\s*=\s*\[[^\]]*"protocol-asset"/s);
  });

  it("allows imported background images from the resolved profile directory", () => {
    const libSource = readFileSync(resolve(process.cwd(), "src-tauri", "src", "lib.rs"), "utf8");

    expect(libSource).toContain("asset_protocol_scope()");
    expect(libSource).toContain("allow_directory(&backgrounds_dir, true)");
  });
});
