import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";

describe("macOS release configuration", () => {
  it("keeps release version metadata in sync", () => {
    const packageJson = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8"));
    const tauriConfig = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );
    const cargoToml = readFileSync(resolve(process.cwd(), "src-tauri", "Cargo.toml"), "utf8");
    const changelog = readFileSync(resolve(process.cwd(), "CHANGELOG.md"), "utf8");
    const releaseWorkflow = readFileSync(resolve(process.cwd(), ".github", "workflows", "release.yml"), "utf8");
    const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

    expect(packageJson.version).toBe("0.0.3");
    expect(tauriConfig.version).toBe(packageJson.version);
    expect(cargoVersion).toBe(packageJson.version);
    expect(changelog).toContain(`## [${packageJson.version}] - `);
    expect(changelog).toContain(`[Unreleased]: https://github.com/QingJ01/Pebble/compare/v${packageJson.version}...HEAD`);
    expect(releaseWorkflow).toContain(`default: v${packageJson.version}`);
  });

  it("defines explicit desktop build scripts for Windows and macOS bundles", () => {
    const packageJson = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8"));

    expect(packageJson.scripts["build:windows"]).toBeTypeOf("string");
    expect(packageJson.scripts["build:macos"]).toBeTypeOf("string");
    expect(packageJson.scripts["build:windows"]).toContain("--bundles nsis");
    expect(packageJson.scripts["build:macos"]).toContain("--bundles app,dmg");
  });

  it("routes the generic build command to platform-specific bundles", async () => {
    const packageJson = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8"));
    const buildScriptPath = resolve(process.cwd(), "scripts", "build-tauri.mjs");
    const buildScriptSource = readFileSync(buildScriptPath, "utf8");
    const buildScript = await import(pathToFileURL(buildScriptPath).href);

    expect(packageJson.scripts.build).toBe("node scripts/build-tauri.mjs");
    expect(buildScriptSource).not.toMatch(/^#!/);
    expect(buildScript.bundleTargetsForPlatform("win32")).toBe("nsis");
    expect(buildScript.bundleTargetsForPlatform("darwin")).toBe("app,dmg");
    expect(() => buildScript.bundleTargetsForPlatform("linux")).toThrow("Unsupported desktop package platform");
  });

  it("keeps Windows notification click helpers out of non-Windows builds", () => {
    const indexingSource = readFileSync(
      resolve(process.cwd(), "src-tauri", "src", "commands", "indexing.rs"),
      "utf8",
    ).replace(/\r\n/g, "\n");
    const eventsSource = readFileSync(resolve(process.cwd(), "src-tauri", "src", "events.rs"), "utf8").replace(
      /\r\n/g,
      "\n",
    );

    expect(indexingSource).toContain("#[cfg(any(windows, test))]\nfn notification_open_payload");
    expect(indexingSource).toContain("#[cfg(windows)]\nfn open_message_from_notification");
    expect(eventsSource).toContain("#[cfg(windows)]\npub const MAIL_NOTIFICATION_OPEN");
  });

  it("includes a macOS icon in the Tauri bundle config", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );

    expect(config.bundle.icon).toContain("icons/icon.icns");
    expect(existsSync(resolve(process.cwd(), "src-tauri", "icons", "icon.icns"))).toBe(true);
  });

  it("enables native credential storage backends for Windows and macOS", () => {
    const cargoToml = readFileSync(resolve(process.cwd(), "Cargo.toml"), "utf8");

    expect(cargoToml).toContain('features = ["apple-native", "windows-native"]');
  });

  it("runs package builds on Windows and macOS in CI", () => {
    const ciWorkflow = readFileSync(resolve(process.cwd(), ".github", "workflows", "ci.yml"), "utf8");

    expect(ciWorkflow).toContain("windows-latest");
    expect(ciWorkflow).toContain("macos-15");
    expect(ciWorkflow).toContain("pnpm ${{ matrix.build_script }}");
    expect(ciWorkflow).toContain("build:windows");
    expect(ciWorkflow).toContain("build:macos");
  });

  it("uploads unsigned macOS DMG artifacts during tagged releases", () => {
    const releaseWorkflow = readFileSync(
      resolve(process.cwd(), ".github", "workflows", "release.yml"),
      "utf8",
    );

    expect(releaseWorkflow).toContain("macOS Release");
    expect(releaseWorkflow).toContain("runs-on: ${{ matrix.os }}");
    expect(releaseWorkflow).toContain("macos-15");
    expect(releaseWorkflow).toContain("macos-15-intel");
    expect(releaseWorkflow).toContain("aarch64-apple-darwin");
    expect(releaseWorkflow).toContain("x86_64-apple-darwin");
    expect(releaseWorkflow).toContain("pnpm tauri build --target ${{ matrix.target }} --bundles app,dmg");
    expect(releaseWorkflow).toContain("target/${{ matrix.target }}/release/bundle/dmg");
    expect(releaseWorkflow).toContain("pebble-macos-${{ matrix.arch }}-${{ env.PEBBLE_VERSION }}");
  });
});
